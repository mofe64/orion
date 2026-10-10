import { useCallback, useMemo, useState } from "react";
import { visibleFeedback, type ScopedFeedback } from "../lib/feedback";

export function useScopedNotice(page: string, connection: unknown) {
  const [notice, show] = useScopedFeedback<string>(page, connection);
  return [notice ?? "", show] as const;
}

export function useScopedFeedback<T>(page: string, connection: unknown) {
  const scope = useMemo(() => ({}), [page, connection]);
  const [feedback, setFeedback] = useState<ScopedFeedback<object, T> | null>(null);
  const show = useCallback((value: T) => setFeedback({ scope, value }), [scope]);
  return [visibleFeedback(feedback, scope), show] as const;
}
