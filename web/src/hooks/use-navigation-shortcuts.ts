import { useEffect, useLayoutEffect, useRef } from 'react';
import { useLocation, useNavigate, useNavigationType } from 'react-router-dom';

/** Scroll position per history entry, so Back returns to where you were. */
const positions = new Map<string, number>();

/**
 * New pages start at the top (or at their #anchor), Back and Forward restore the position,
 * and "/" jumps to search from anywhere. A layout effect, so it runs before page effects that
 * reveal a passage.
 */
export function useNavigationShortcuts() {
  const { key, hash } = useLocation();
  const type = useNavigationType();
  const navigate = useNavigate();
  const current = useRef(key);
  useEffect(() => {
    const save = () => positions.set(current.current, window.scrollY);
    window.addEventListener('scroll', save, { passive: true });
    return () => window.removeEventListener('scroll', save);
  }, []);
  useLayoutEffect(() => {
    current.current = key;
    const target = hash && document.getElementById(hash.slice(1));
    if (target) target.scrollIntoView();
    else window.scrollTo(0, type === 'POP' ? (positions.get(key) ?? 0) : 0);
  }, [key, hash, type]);
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement;
      if (event.key !== '/' || event.metaKey || event.ctrlKey) return;
      if (target.closest('input, textarea, select, [contenteditable]')) return;
      event.preventDefault();
      const input = document.getElementById('search-input');
      if (input) input.focus();
      else navigate('/search');
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [navigate]);
}
