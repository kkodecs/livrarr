import { useEffect, useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { getLanguages } from "@/api";
import { SUPPORTED_LANGUAGES } from "@/types/api";

/**
 * The search languages the admin enabled, read from the language route that
 * every signed-in user may call, and the language a search is sent in.
 *
 * `enabled` is the supported languages whose codes are in the saved list, or
 * English alone while there is no list (not loaded yet, or unreadable).
 *
 * `selected` starts at `addressLanguage` or English. Whenever the saved list
 * is read with different contents, it returns to the list's first code, so a
 * choice the list no longer offers is never kept. An `addressLanguage` wins
 * over the list. A re-read with the same contents keeps the user's choice.
 */
export function useEnabledLanguages(addressLanguage = "") {
  const { data } = useQuery({
    queryKey: ["enabled-languages"],
    queryFn: getLanguages,
  });

  const savedKey = data ? data.languages.join(",") : undefined;

  const enabled = useMemo(() => {
    const codes = savedKey === undefined ? ["en"] : savedKey.split(",");
    return SUPPORTED_LANGUAGES.filter((l) => codes.includes(l.code));
  }, [savedKey]);

  const [selected, setSelected] = useState(addressLanguage || "en");

  useEffect(() => {
    if (addressLanguage) {
      setSelected(addressLanguage);
    } else if (savedKey !== undefined) {
      setSelected(savedKey.split(",")[0] || "en");
    }
  }, [addressLanguage, savedKey]);

  return { enabled, selected, setSelected };
}
