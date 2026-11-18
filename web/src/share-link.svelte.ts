import type { Share } from './api';

/** A recording's link as the owner's page offers it: the saved link while it works by this
 * page's clock. The clock moves on when the link expires, when the page is shown again, and on
 * `check()`, which the page calls before it shows or hands out the link, since a hidden page's
 * timers run late. When the link lapses the page asks the server again (`refresh`), which may
 * have extended or replaced it. The server decides; this clock can differ from its by their skew. */
export class ShareLink {
  #now = $state(Date.now());
  #share: () => Share | null | undefined;

  constructor(share: () => Share | null | undefined, refresh: () => unknown) {
    this.#share = share;
    $effect(() => {
      const at = this.#share()?.expires_at;
      if (!at) return;
      const deadline = Date.parse(at);
      let timer: ReturnType<typeof setTimeout> | undefined;
      const tick = () => {
        clearTimeout(timer);
        // One reading decides both what is offered and when to look again.
        const now = (this.#now = Date.now());
        // A timer waits at most about 24 days; a link further out is checked again then.
        if (now < deadline) timer = setTimeout(tick, Math.min(deadline - now, 2 ** 31 - 1));
        else refresh();
      };
      tick();
      document.addEventListener('visibilitychange', tick);
      return () => {
        clearTimeout(timer);
        document.removeEventListener('visibilitychange', tick);
      };
    });
  }

  /** The link while it works, or null. */
  get link(): Share | null {
    const share = this.#share();
    if (!share) return null;
    return share.expires_at && Date.parse(share.expires_at) <= this.#now ? null : share;
  }

  /** Moves the page's clock to now. */
  check() {
    this.#now = Date.now();
  }
}
