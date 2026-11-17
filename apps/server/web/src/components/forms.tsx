export const inputClass =
  "mt-2 block w-full rounded-lg border border-muted/40 bg-background px-3 py-2.5 text-sm outline-offset-4 focus:outline-accent disabled:opacity-50";
export const buttonClass =
  "cursor-pointer rounded-lg bg-primary px-4 py-2.5 text-sm font-medium text-background hover:bg-primary/85 disabled:cursor-not-allowed disabled:opacity-40";
export const secondaryClass =
  "cursor-pointer text-sm underline decoration-muted/50 underline-offset-4 disabled:cursor-not-allowed disabled:opacity-40";

export function ActionError({ error }: { error: Error | null }) {
  return error ? (
    <p role="alert" className="mt-3 text-sm leading-6 text-error">
      {error.message}
    </p>
  ) : null;
}
