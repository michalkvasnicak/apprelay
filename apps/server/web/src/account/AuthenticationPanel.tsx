import { useRef } from "react";

import { Pending, Status, buttonClass, inputClass, secondaryClass } from "../components/forms";
import { accountController } from "./controller";
import { inputValue, useAccountAction, useAccountBusy, type AccountSnapshot } from "./hooks";

function Login() {
  const form = useRef<HTMLFormElement>(null);
  const busy = useAccountBusy();
  const login = useAccountAction(() => {
    const username = inputValue(form, "username").trim();
    const password = inputValue(form, "password");
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
          Password
          <input
            name="password"
            type="password"
            autoComplete="current-password"
            required
            maxLength={1024}
            className={inputClass}
          />
        </label>
        <button type="submit" className={buttonClass + " w-full"}>
          <Pending active={login.isPending} label="Signing in…">
            Sign in
          </Pending>
        </button>
        <p className="text-center text-xs text-muted">Your password is never stored.</p>
      </fieldset>
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
    <div className="space-y-5">
      <p className="text-sm text-muted">Enter the code sent to your trusted Apple device.</p>
      <form
        ref={form}
        onSubmit={(event) => {
          event.preventDefault();
          verify.mutate();
        }}
      >
        <fieldset disabled={busy} className="space-y-4">
          <input
            name="code"
            aria-label="Verification code"
            placeholder="000000"
            inputMode="numeric"
            autoComplete="one-time-code"
            pattern="[0-9]{6}"
            maxLength={6}
            required
            autoFocus
            className={inputClass + " text-center font-mono text-2xl tracking-[0.4em]"}
          />
          <button type="submit" className={buttonClass + " w-full"}>
            <Pending active={verify.isPending} label="Verifying…">
              Verify
            </Pending>
          </button>
        </fieldset>
      </form>
      <div className="flex gap-3">
        <button
          type="button"
          disabled={busy}
          onClick={() => resend.mutate()}
          className={secondaryClass}
        >
          <Pending active={resend.isPending} label="Sending…">
            Resend code
          </Pending>
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
    </div>
  );
}

function TeamSelection({ account }: { account: AccountSnapshot }) {
  const busy = useAccountBusy();
  const choose = useAccountAction((teamId: string) => accountController.selectTeam(teamId));
  const load = useAccountAction(() => accountController.loadTeams());
  const reset = useAccountAction(() => accountController.forget());
  if (!account.teams) {
    return (
      <div className="space-y-4">
        <p className="text-sm text-muted">Signed in, but teams could not be loaded.</p>
        <div className="flex flex-wrap gap-3">
          <button
            type="button"
            disabled={busy}
            onClick={() => load.mutate()}
            className={buttonClass}
          >
            <Pending active={load.isPending} label="Loading teams…">
              Retry
            </Pending>
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
      </div>
    );
  }
  return (
    <div className="space-y-3">
      <label className="block text-sm font-medium">
        Development team
        <select
          value={account.teamId ?? ""}
          disabled={busy}
          onChange={(event) => choose.mutate(event.currentTarget.value)}
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
      {choose.isPending && <Status>Selecting team…</Status>}
      {account.teams.length === 0 && (
        <p className="text-sm text-error">No development teams found for this account.</p>
      )}
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
    return <p className="text-sm text-muted">Your Apple account is saved on this device.</p>;
  }
  return <Login />;
}
