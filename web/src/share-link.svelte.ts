/** The owner's pages decide by their own clock whether a share link still works, so an expired
 * link is never offered or handed out. The clock moves on when a link expires, when the page is
 * shown again, and on `check()`, which a page calls before it shows or hands out a link, since a
 * hidden page's timers run late. When a link lapses the page asks the server again (`lapsed`),
 * which may have extended or replaced it. The server decides; this clock can differ from its by
 * their skew. */
export class LinkClock {
  #now = $state(Date.now());

  /** `expiries`: when each link the page shows stops working (null: never). */
  constructor(expiries: () => (string | null | undefined)[], lapsed: () => unknown) {
    $effect(() => {
      const deadlines = expiries().flatMap((at) => (at ? [Date.parse(at)] : []));
      if (!deadlines.length) return;
      let timer: ReturnType<typeof setTimeout> | undefined;
      let gone = 0;
      const tick = () => {
        clearTimeout(timer);
        // One reading decides what works, whether a link lapsed, and when to look again.
        const now = (this.#now = Date.now());
        const passed = deadlines.filter((deadline) => deadline <= now).length;
        if (passed > gone) lapsed();
        gone = passed;
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

  /** `link` while it works by the page's clock, or null. */
  offered<T extends { expires_at: string | null }>(link: T | null | undefined): T | null {
    return link && this.works(link.expires_at) ? link : null;
  }

  /** Moves the page's clock to now. */
  check() {
    this.#now = Date.now();
  }
}
