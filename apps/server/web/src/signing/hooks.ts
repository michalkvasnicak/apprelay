import { useQuery } from "@tanstack/react-query";

import { signingController } from "./controller";

export function useSigning() {
  return useQuery({
    queryKey: ["signing"],
    queryFn: () => signingController.snapshot(),
    staleTime: Infinity,
    retry: false,
    refetchOnWindowFocus: false,
  });
}
