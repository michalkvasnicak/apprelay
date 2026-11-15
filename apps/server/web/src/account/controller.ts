import { Anisette } from "../apple/anisette";
import { signIn, trustedDevice, type SignInResult } from "../apple/authentication";
import { DeveloperPortal } from "../apple/developer";
import { ensureCertificate, provisionApp } from "../apple/provisioning";
import { certificateExpiry, matchingCertificate } from "../apple/signing-assets";
import type { StoredAccount, Team } from "../apple/types";
import { LocalVault } from "../storage/vault";

export class AccountError extends Error {}

type PendingChallenge = Extract<SignInResult, { type: "two-factor" }> & {
  deadline: number;
  resendAt: number;
};

class AccountController {
  private readonly vault = new LocalVault();
  private account?: StoredAccount;
  private anisette?: Anisette;
  private teams: Team[] = [];
  private challenge?: PendingChallenge;
  private busy = false;

  snapshot() {
    const identity = this.account?.identity;
    const profile = this.account?.profile;
    return {
      status: this.challenge
        ? ("two-factor" as const)
        : this.account
          ? ("authenticated" as const)
          : ("signed-out" as const),
      teams: this.teams,
      teamId: this.account?.teamId,
      vaultExists: this.vault.exists(),
      vaultUnlocked: this.vault.unlocked,
      certificateRequested: identity?.submitted ?? false,
      identityTeamId: identity?.teamId,
      certificateExpiresAt: identity?.certificate
        ? certificateExpiry(identity.certificate)
        : undefined,
      profile: profile
        ? { bundleId: profile.bundleId, udid: profile.udid, expiresAt: profile.expiresAt }
        : undefined,
    };
  }

  // Serialize commands. Errors exposed to React/Query never contain foreign
  // causes, protocol payloads, credentials, mutation variables, or private keys.
  private async run(message: string, operation: () => Promise<void>) {
    if (this.busy) {
      throw new AccountError("Wait for the current operation to finish");
    }
    this.busy = true;
    try {
      await operation();
    } catch (error) {
      if (error instanceof AccountError) {
        throw error;
      }
      throw new AccountError(message);
    } finally {
      this.busy = false;
    }
  }

  private portal() {
    if (!this.account || !this.anisette) {
      throw new AccountError("Sign in to Apple first");
    }
    return new DeveloperPortal(this.account.session, this.anisette);
  }

  private async accept(result: SignInResult, anisette: Anisette) {
    if (result.type === "two-factor") {
      this.challenge = { ...result, deadline: Date.now() + 600_000, resendAt: Date.now() + 60_000 };
      this.anisette = anisette;
      if (!(await trustedDevice(result.challenge))) {
        this.challenge = undefined;
        this.anisette = undefined;
        throw new AccountError("Apple did not confirm code delivery. Start a new sign-in");
      }
      return;
    }
    const teams = await new DeveloperPortal(result.session, anisette).teams();
    this.account = {
      session: result.session,
      anisette: anisette.snapshot(),
      teamId: teams.length === 1 ? teams[0].id : undefined,
    };
    this.teams = teams;
    this.anisette = anisette;
    this.challenge = undefined;
  }

  login(username: string, password: string) {
    return this.run(
      "Apple sign-in failed. Check your account, relay, and anisette provider, then start again",
      async () => {
        if (this.vault.exists() || this.account || this.challenge) {
          throw new AccountError("Unlock the saved account or reset the current sign-in first");
        }
        const anisette = await Anisette.create();
        try {
          await this.accept(await signIn(anisette, username, password), anisette);
        } catch (error) {
          this.challenge = undefined;
          this.anisette = undefined;
          throw error;
        }
      },
    );
  }

  private activeChallenge() {
    if (!this.challenge || Date.now() > this.challenge.deadline) {
      this.challenge = undefined;
      this.anisette = undefined;
      throw new AccountError("Verification expired. Start a new sign-in");
    }
    return this.challenge;
  }

  verify(code: string) {
    return this.run(
      "Verification failed. Start a new sign-in; the code was not replayed",
      async () => {
        const pending = this.activeChallenge();
        if (!/^\d{6}$/.test(code)) {
          throw new AccountError("Enter a six-digit code");
        }
        let accepted: boolean;
        try {
          accepted = await trustedDevice(pending.challenge, code);
        } catch (error) {
          this.challenge = undefined;
          this.anisette = undefined;
          throw error;
        }
        if (!accepted) {
          throw new AccountError("The verification code was not accepted. Try another code");
        }
        const anisette = this.anisette;
        this.challenge = undefined; // Never submit an accepted code a second time.
        this.anisette = undefined;
        if (!anisette) {
          throw new AccountError("Start a new sign-in");
        }
        await this.accept(await pending.resume(), anisette);
      },
    );
  }

  resend() {
    return this.run("Apple could not send another code. Start a new sign-in", async () => {
      const pending = this.activeChallenge();
      if (Date.now() < pending.resendAt) {
        throw new AccountError("Wait 60 seconds between code requests");
      }
      pending.resendAt = Date.now() + 60_000;
      try {
        if (!(await trustedDevice(pending.challenge))) {
          throw new Error("Delivery not confirmed");
        }
      } catch (error) {
        this.challenge = undefined;
        this.anisette = undefined;
        throw error;
      }
    });
  }

  unlock(passphrase: string) {
    return this.run(
      "Could not unlock or validate the saved Apple session. Check the passphrase and connection; stored data was kept",
      async () => {
        try {
          const account = await this.vault.unlock(passphrase);
          if (account.session.expiresAt !== undefined && account.session.expiresAt <= Date.now()) {
            throw new AccountError(
              "The saved Apple session expired. Stored signing assets were kept; session renewal is not implemented yet",
            );
          }
          const anisette = Anisette.restore(account.anisette);
          const teams = await new DeveloperPortal(account.session, anisette).teams();
          if (
            account.identity?.certificate &&
            !matchingCertificate(account.identity.certificate, account.identity)
          ) {
            throw new AccountError(
              "The saved certificate is expired or does not match its key. Stored data was kept",
            );
          }
          account.teamId = teams.some((team) => team.id === account.teamId)
            ? account.teamId
            : undefined;
          this.account = account;
          this.anisette = anisette;
          this.teams = teams;
        } catch (error) {
          this.vault.lock();
          throw error;
        }
      },
    );
  }

  save(passphrase: string) {
    return this.run(
      "Could not save the encrypted vault. Use at least 12 characters and check browser storage permissions",
      async () => {
        if (!this.account) {
          throw new AccountError("Sign in first");
        }
        await this.vault.create(passphrase, this.account);
      },
    );
  }

  selectTeam(teamId: string) {
    return this.run("Could not save the selected team", async () => {
      if (!this.account || !this.teams.some((team) => team.id === teamId)) {
        throw new AccountError("Choose an available team");
      }
      if (this.account.identity && this.account.identity.teamId !== teamId) {
        throw new AccountError(
          "This vault already contains a key for another team. Keep that team selected to preserve its assets",
        );
      }
      this.account.teamId = teamId;
      if (this.vault.unlocked) {
        await this.vault.save(this.account);
      }
    });
  }

  certificate() {
    return this.run(
      "Certificate preparation failed or is pending. The key was kept; recheck reconciles existing certificates without replaying an uncertain request. Apple quotas may require manual action",
      async () => {
        const portal = this.portal();
        if (!this.account || !this.vault.unlocked) {
          throw new AccountError(
            "Save and unlock the encrypted vault before creating a signing key",
          );
        }
        if (!this.account.teamId) {
          throw new AccountError("Choose a team first");
        }
        const account = this.account;
        await ensureCertificate(account, portal, () => this.vault.save(account));
      },
    );
  }

  provision(bundleId: string) {
    return this.run(
      "Provisioning failed or its outcome is unknown. Nothing was retried automatically; a new explicit attempt reconciles devices and app IDs but may regenerate a profile",
      async () => {
        const portal = this.portal();
        if (!this.account || !this.vault.unlocked) {
          throw new AccountError("Unlock the vault first");
        }
        const account = this.account;
        await provisionApp(account, portal, bundleId, () => this.vault.save(account));
      },
    );
  }

  lock() {
    return this.run("Could not lock the account", async () => {
      if (this.account && !this.vault.exists()) {
        throw new AccountError("Save the account before locking, or explicitly forget it");
      }
      this.clearMemory();
    });
  }

  forget() {
    return this.run(
      "Could not remove browser data. Check browser storage permissions",
      async () => {
        this.vault.forget();
        this.clearMemory();
      },
    );
  }

  private clearMemory() {
    this.account = undefined;
    this.anisette = undefined;
    this.challenge = undefined;
    this.teams = [];
    this.vault.lock();
  }
}

export const accountController = new AccountController();
