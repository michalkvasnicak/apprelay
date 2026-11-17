import { useRef } from "react";

import { ActionError, buttonClass, inputClass, secondaryClass } from "../components/forms";
import { accountController } from "./controller";
import { inputValue, useAccountAction, useAccountBusy, type AccountSnapshot } from "./hooks";

function Login() {
  const form = useRef<HTMLFormElement>(null);
  const busy = useAccountBusy();
  const login = useAccountAction(() => {
    const username = inputValue(form, "username").trim();
    const password = inputValue(form, "password", true);
    return accountController.login(username, password);
  });

  return (
    <form
      ref={form}
      onSubmit={(event) => {
        event.preventDefault();
        login.mutate();
      }}
    >
      <fieldset disabled={busy} className="space-y-4">
        <label className="block text-sm font-medium">
          Apple account
          <input
            name="username"
            autoComplete="username"
            required
            maxLength={320}
            className={inputClass}
          />
        </label>
        <label className="block text-sm font-medium">
          Apple password
          <input
            name="password"
            type="password"
            autoComplete="current-password"
            required
            maxLength={1024}
            className={inputClass}
          />
        </label>
        <p className="text-xs leading-5 text-muted">
          Your Apple password is used only on this device for sign-in and is never stored. Your
          saved session stays on this device; the HTTPS relay can read Apple session tokens in
          transit.
        </p>
        <button type="submit" className={buttonClass}>
          {login.isPending ? "Signing in…" : "Sign in to Apple"}
        </button>
      </fieldset>
      <ActionError error={login.error} />
    </form>
  );
}

function TwoFactor() {
  const form = useRef<HTMLFormElement>(null);
  const busy = useAccountBusy();
  const verify = useAccountAction(() => accountController.verify(inputValue(form, "code", true)));
  const resend = useAccountAction(() => accountController.resend());
  const reset = useAccountAction(() => accountController.forget());
  return (
    <div>
      <p className="mb-5 text-sm leading-6 text-muted">
        Enter the six-digit code sent to your trusted Apple device. This challenge expires after ten
        minutes. SMS is not supported yet.
      </p>
      <form
        ref={form}
        onSubmit={(event) => {
          event.preventDefault();
          verify.mutate();
        }}
      >
        <fieldset disabled={busy} className="space-y-4">
          <label className="block text-sm font-medium">
            Verification code
            <input
              name="code"
              inputMode="numeric"
              autoComplete="one-time-code"
              pattern="[0-9]{6}"
              maxLength={6}
              required
              className={inputClass}
            />
          </label>
          <button type="submit" className={buttonClass}>
            {verify.isPending ? "Verifying…" : "Verify code"}
          </button>
        </fieldset>
      </form>
      <div className="mt-5 flex gap-5">
        <button
          type="button"
          disabled={busy}
          onClick={() => resend.mutate()}
          className={secondaryClass}
        >
          Resend code
        </button>
        <button
          type="button"
          disabled={busy}
          onClick={() => reset.mutate()}
          className={secondaryClass}
        >
          Start over
        </button>
      </div>
      <ActionError error={verify.error ?? resend.error ?? reset.error} />
    </div>
  );
}

function TeamSelection({ account }: { account: AccountSnapshot }) {
  const select = useRef<HTMLSelectElement>(null);
  const busy = useAccountBusy();
  const choose = useAccountAction(() => accountController.selectTeam(select.current?.value ?? ""));
  return (
    <div>
      <p className="mb-5 text-sm text-primary">Signed in to Apple.</p>
      <label className="block text-sm font-medium">
        Development team
        <select
          ref={select}
          value={account.teamId ?? ""}
          disabled={busy}
          onChange={() => choose.mutate()}
          className={inputClass}
        >
          <option value="" disabled>
            Choose a team
          </option>
          {account.teams.map((team) => (
            <option key={team.id} value={team.id}>
              {team.name} · {team.kind}
            </option>
          ))}
        </select>
      </label>
      {account.teams.length === 0 && (
        <p className="mt-3 text-sm text-error">
          Apple returned no development teams for this account.
        </p>
      )}
      <p className="mt-3 text-xs leading-5 text-muted">
        Personal Teams have short-lived profiles and restrictive quotas and capabilities. Existing
        certificates are never revoked automatically.
      </p>
      <ActionError error={choose.error} />
    </div>
  );
}

export function AuthenticationPanel({ account }: { account: AccountSnapshot }) {
  if (account.status === "two-factor") {
    return <TwoFactor />;
  }
  if (account.status === "authenticated") {
    return <TeamSelection account={account} />;
  }
  if (account.vaultExists) {
    return (
      <p className="text-sm leading-6 text-muted">
        Your Apple account is saved on this device. Unlock it below to continue.
      </p>
    );
  }
  return <Login />;
}
