import { useInfiniteQuery } from '@tanstack/react-query';
import { api, isProcessing, type Entry, type Page } from '../api';

/** Paged library listing. `params` is the extra query string; polls only while one is processing. */
export function useEntries(params: string, poll: boolean) {
  return useInfiniteQuery({
    queryKey: ['entries', params],
    initialPageParam: 0,
    queryFn: ({ pageParam }) =>
      api<Page<'entries', Entry>>(`/v1/entries?cursor=${pageParam}${params}`),
    getNextPageParam: (last) => last.next_cursor ?? undefined,
    // Pages are fetched by position, so a recording arriving in between repeats a row.
    select: (data) => {
      const seen = new Set<string>();
      return data.pages
        .flatMap((p) => p.entries)
        .filter((e) => !seen.has(e.id) && !!seen.add(e.id));
    },
    // ponytail: refetches every loaded page while one is processing; poll page one only if many are loaded.
    refetchInterval: (q) =>
      poll && q.state.data?.pages.some((p) => p.entries.some(isProcessing)) ? 4000 : false,
  });
}
