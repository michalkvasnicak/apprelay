import { useIsMutating, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type { RefObject } from "react";

import { AccountError, accountController } from "./controller";

export function useAccount() {
  return useQuery({
    queryKey: ["account"],
    queryFn: () => accountController.snapshot(),
    retry: false,
    staleTime: Infinity,
    refetchOnWindowFocus: false,
  });
}

export type AccountSnapshot = ReturnType<typeof accountController.snapshot>;

export function useAccountAction(action: () => Promise<void>) {
  const client = useQueryClient();
  return useMutation({
    mutationKey: ["account-command"],
    // No mutation variables/results: secrets stay in the operation, not Query.
    mutationFn: async () => {
      try {
        await action();
      } catch (error) {
        throw new AccountError(
          error instanceof AccountError
            ? error.message
            : "The operation failed. No request was retried automatically",
        );
      }
    },
    retry: false,
    onSettled: () => client.invalidateQueries({ queryKey: ["account"] }),
    gcTime: 0,
  });
}

export function useAccountBusy() {
  return useIsMutating({ mutationKey: ["account-command"] }) > 0;
}

export function inputValue(form: RefObject<HTMLFormElement | null>, name: string, clear = false) {
  const field = form.current?.elements.namedItem(name);
  if (!(field instanceof HTMLInputElement)) {
    throw new AccountError("Form is unavailable");
  }
  const value = field.value;
  if (clear) {
    field.value = "";
  }
  return value;
}
