import { SvelteMap } from 'svelte/reactivity';

/** Links taken for gone, by URL, with when (`moment`): this page stopped them or moved their
 * recording to Trash, the server answered 404 for them, or a read no longer returned them. A
 * token never works again once it is gone, so no page offers these, even before it hears what
 * (if anything) replaced them; `confirm` corrects one a racing read took for gone. */
const dead = new SvelteMap<string, number>();
/** This page's share events, in order: each link taken for gone and each read or change that
 * leaves. */
let moment = 0;
/** This link is gone (stopped, deleted by Trash, answered 404, or no longer read back). */
export const forget = (url: string) => {
  dead.set(url, ++moment);
};
/** A read or a change leaves: the moment to `confirm` what its answer returns against. */
export const leaving = () => ++moment;
/** A read or change that left at `since` was answered with this link working. If the link was
 * taken for gone before that left, the read that did so had raced a change at the server (an
 * extension at the deadline, say): the link works and is offered again. One taken for gone after
 * it left stays gone, as the answer may have been served before the link was stopped. */
export function confirm(url: string, since: number) {
  seen.set(url, Math.max(seen.get(url) ?? 0, since));
  const at = dead.get(url);
  if (at !== undefined && at < since) dead.delete(url);
}
/** For each link, when the latest read or change answered with it working left. */
const seen = new Map<string, number>();
/** A read that left at `since` no longer had this link: it is gone, unless a read or change that
 * left after it was answered with the link working. Either answer may be the newer one at the
 * server; the link stays offered until the page's next read settles it. */
export function missing(url: string, since: number) {
  if ((seen.get(url) ?? 0) >= since) return;
  // Marked as of when that read left, not when it landed, so a read that left after it and finds
  // the link working still brings it back (`confirm`), whichever lands first; never earlier than a
  // mark already there (a stop or a 404 comes after the server committed it).
  dead.set(url, Math.max(dead.get(url) ?? 0, since));
}

/** The owner's pages decide by their own clock whether a share link still works, so an expired
 * link is never offered or handed out. The clock moves on when a link expires, when the page is
 * shown again, and on `check()`, which a page calls before it shows or hands out a link, since a
 * hidden page's timers run late. When a link lapses the page asks the server again (`lapsed`),
 * which may have extended or replaced it. The server decides; this clock can differ from its by
 * their skew. */
export class LinkClock {
  #now = $state(Date.now());
  /** Deadlines the page has already asked the server about. Every answer, a failed one too,
   * reruns what reads the links, so this outlives those reruns: a lapsed link is asked about
   * once, not again on each failed answer. */
  #asked = new Set<number>();
  /** The running check, while the page shows a link that expires. */
  #tick: (() => void) | undefined;

  /** `expiries`: when each link the page shows stops working (null: never). */
  constructor(expiries: () => (string | null | undefined)[], lapsed: () => unknown) {
    $effect(() => {
      const deadlines = expiries().flatMap((at) => (at ? [Date.parse(at)] : []));
      if (!deadlines.length) return;
      let timer: ReturnType<typeof setTimeout> | undefined;
      const tick = () => {
        clearTimeout(timer);
        // One reading decides what works, whether a link lapsed, and when to look again.
        const now = (this.#now = Date.now());
        const lapsedNow = deadlines.filter((d) => d <= now && !this.#asked.has(d));
        for (const deadline of lapsedNow) this.#asked.add(deadline);
        if (lapsedNow.length) lapsed();
        const next = Math.min(...deadlines.filter((deadline) => deadline > now));
        // A timer waits at most about 24 days; a link further out is checked again then.
        if (next < Infinity) timer = setTimeout(tick, Math.min(next - now, 2 ** 31 - 1));
      };
      this.#tick = tick;
      tick();
      document.addEventListener('visibilitychange', tick);
      return () => {
        clearTimeout(timer);
        document.removeEventListener('visibilitychange', tick);
        if (this.#tick === tick) this.#tick = undefined;
      };
    });
  }

  /** Whether a link that stops working at `at` (null: never) works by the page's clock. */
  works(at: string | null) {
    return !at || Date.parse(at) > this.#now;
  }

  /** `link` while it works by the page's clock and the server has not said it is gone, or null. */
  offered<T extends { url: string; expires_at: string | null }>(
    link: T | null | undefined,
  ): T | null {
    return link && !dead.has(link.url) && this.works(link.expires_at) ? link : null;
  }

  /** Moves the page's clock to now, asking the server if a link has lapsed meanwhile; says
   * whether it asked. */
  check() {
    const asked = this.#asked.size;
    if (this.#tick) this.#tick();
    else this.#now = Date.now();
    return this.#asked.size > asked;
  }
}
