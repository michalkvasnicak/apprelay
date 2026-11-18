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

// Keep the sanitized error visible even when the initiating panel unmounts.
const accountErrorKey = ["account-command-error"] as const;

export function useAccountError() {
  return useQuery({
    queryKey: accountErrorKey,
    queryFn: (): AccountError | null => null,
    initialData: null,
    staleTime: Infinity,
  }).data;
}

export function useAccountAction<T = void>(action: (input: T) => Promise<void>) {
  const client = useQueryClient();
  return useMutation({
    mutationKey: ["account-command"],
    // Query retains mutation variables, so pass only non-secret inputs.
    mutationFn: async (input: T) => {
      try {
        await action(input);
      } catch (error) {
        throw new AccountError(
          error instanceof AccountError
            ? error.message
            : "The operation failed. No request was retried automatically",
        );
      }
    },
    retry: false,
    onMutate: () => {
      client.setQueryData(accountErrorKey, null);
    },
    onError: (error) => {
      client.setQueryData(accountErrorKey, error);
    },
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
