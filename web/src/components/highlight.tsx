import { Fragment, useMemo } from 'react';

/** Wraps the query's words in <mark>, case-insensitively. */
export function Highlight({ text, query }: { text: string; query: string }) {
  const pattern = useMemo(() => {
    const words = query
      .split(/[^\p{L}\p{N}]+/u)
      .filter((w) => w.length > 1)
      .map((w) => w.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'));
    return words.length ? new RegExp(`(${words.join('|')})`, 'giu') : null;
  }, [query]);
  if (!pattern) return <>{text}</>;
  return (
    <>
      {text.split(pattern).map((part, i) => (
        <Fragment key={i}>{i % 2 ? <mark>{part}</mark> : part}</Fragment>
      ))}
    </>
  );
}
