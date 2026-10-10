export interface Failure { message: string; detail: string }

export class OwnerRequestError extends Error {
  readonly detail: string;
  constructor(message: string, error: unknown) {
    super(message);
    this.detail = error instanceof Error ? error.message : String(error);
  }
}

export async function forOwner<T>(work: () => Promise<T>, message: string): Promise<T> {
  try { return await work(); }
  catch (error) { throw new OwnerRequestError(message, error); }
}

export function failure(message: string, error: unknown): Failure {
  return { message, detail: error instanceof OwnerRequestError ? error.detail : error instanceof Error ? error.message : String(error) };
}

export interface ScopedFeedback<S, V> { scope: S; value: V }

// Scope identity changes on navigation or connection change. Late responses from
// the old scope cannot put its notice back on the new page, even after a revisit.
export function visibleFeedback<S, V>(feedback: ScopedFeedback<S, V> | null, scope: S): V | null {
  return feedback?.scope === scope ? feedback.value : null;
}
