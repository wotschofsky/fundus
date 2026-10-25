import { useEffect, useState } from 'react';
import { Check, Copy } from 'lucide-react';

export function CopyField({ value, label }: { value: string; label: string }) {
  const [state, setState] = useState<'idle' | 'copied' | 'failed'>('idle');
  useEffect(() => {
    if (state === 'idle') return;
    const timer = setTimeout(() => setState('idle'), 1800);
    return () => clearTimeout(timer);
  }, [state]);
  return (
    <div className="copy-field">
      <code>{value}</code>
      <button
        type="button"
        className="icon-button"
        aria-label={state === 'copied' ? 'Copied' : label}
        title={state === 'failed' ? 'Copy is unavailable here. Select the text instead.' : label}
        onClick={() =>
          // No clipboard API outside secure origins (plain HTTP on a LAN address).
          (navigator.clipboard?.writeText(value) ?? Promise.reject()).then(
            () => setState('copied'),
            () => setState('failed'),
          )
        }
      >
        {state === 'copied' ? <Check size={16} /> : <Copy size={16} />}
      </button>
    </div>
  );
}
