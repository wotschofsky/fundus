import { SvelteSet } from 'svelte/reactivity';

/** URLs of links that are gone: this page stopped them, or the server answered 404 for them. A
 * token never works again once it is gone, so no page offers these again, even before it hears
 * what (if anything) replaced them. */
const dead = new SvelteSet<string>();
/** This link is gone: stopped here, or the server answered 404 for it. */
export const forget = (url: string) => {
  dead.add(url);
};

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
      tick();
      document.addEventListener('visibilitychange', tick);
      return () => {
        clearTimeout(timer);
        document.removeEventListener('visibilitychange', tick);
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

  /** Moves the page's clock to now. */
  check() {
    this.#now = Date.now();
  }
}
