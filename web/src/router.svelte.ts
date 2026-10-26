import { tick } from 'svelte';

/** Scroll position per history entry, so Back returns to where you were. */
const positions = new Map<string, number>();
const keyOf = (state: unknown) => (state as { key?: string } | null)?.key ?? 'default';

/**
 * History-API routing for the few pages there are. New pages start at the top (or at their
 * #anchor), Back and Forward restore the position, and in-app links navigate without a reload.
 */
class Router {
  url = $state(new URL(location.href));
  /** `default` for the entry the tab opened on; Back from there would leave the app. */
  key = $state(keyOf(history.state));

  constructor() {
    history.scrollRestoration = 'manual';
    addEventListener('scroll', () => positions.set(this.key, scrollY), { passive: true });
    addEventListener('popstate', () =>
      this.#show(new URL(location.href), keyOf(history.state), true),
    );
    document.addEventListener('click', (event) => {
      const link = (event.target as Element).closest?.('a');
      if (!link || event.defaultPrevented || event.button !== 0) return;
      if (event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
      if (link.target || link.hasAttribute('download') || link.origin !== location.origin) return;
      if (link.pathname.startsWith('/api/')) return;
      event.preventDefault();
      this.navigate(link.href);
    });
  }

  get inApp() {
    return this.key !== 'default';
  }

  navigate(to: string, { replace = false } = {}) {
    const url = new URL(to, location.href);
    replace ||= url.href === location.href;
    const key = replace ? this.key : Math.random().toString(36).slice(2, 10);
    history[replace ? 'replaceState' : 'pushState']({ key }, '', url);
    // Before the new page renders, so a page that reveals a passage is not scrolled back up.
    if (!url.hash) scrollTo(0, 0);
    this.#show(url, key, false);
  }

  /** Sets or clears one query parameter. Filters replace the history entry; a search adds one. */
  setParam(name: string, value: string, { replace = true } = {}) {
    const url = new URL(this.url);
    if (value) url.searchParams.set(name, value);
    else url.searchParams.delete(name);
    this.navigate(url.href, { replace });
  }

  #show(url: URL, key: string, back: boolean) {
    this.url = url;
    this.key = key;
    tick().then(() => {
      const target = url.hash && document.getElementById(decodeURIComponent(url.hash.slice(1)));
      if (target) target.scrollIntoView();
      else if (back) scrollTo(0, positions.get(key) ?? 0);
    });
  }
}

export const router = new Router();
