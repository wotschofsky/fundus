import { ChevronDown } from 'lucide-react';

export function LoadMoreButton({
  query,
}: {
  query: { hasNextPage: boolean; isFetchingNextPage: boolean; fetchNextPage: () => unknown };
}) {
  if (!query.hasNextPage) return null;
  return (
    <button
      className="load-more"
      disabled={query.isFetchingNextPage}
      onClick={() => query.fetchNextPage()}
    >
      {query.isFetchingNextPage ? 'Loading…' : 'Show older recordings'}
      <ChevronDown size={16} aria-hidden />
    </button>
  );
}
