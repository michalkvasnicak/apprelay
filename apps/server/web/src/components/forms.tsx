export const inputClass =
  "mt-2 block w-full rounded-lg border border-stone-300 bg-white px-3 py-2.5 text-sm outline-offset-4 focus:outline-emerald-700 disabled:opacity-50";
export const buttonClass =
  "cursor-pointer rounded-lg bg-stone-900 px-4 py-2.5 text-sm font-medium text-white hover:bg-stone-700 disabled:cursor-not-allowed disabled:opacity-40";
export const secondaryClass =
  "cursor-pointer text-sm underline decoration-stone-300 underline-offset-4 disabled:cursor-not-allowed disabled:opacity-40";

export function ActionError({ error }: { error: Error | null }) {
  return error ? (
    <p role="alert" className="mt-3 text-sm leading-6 text-red-800">
      {error.message}
    </p>
  ) : null;
}
