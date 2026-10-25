import { LoaderCircle } from 'lucide-react';
import { isProcessing, type Entry } from '../api';
import { stageName } from '../format';

/** Processing state for a library row; nothing once the recording is ready. */
export function StageLabel({ entry }: { entry: Entry }) {
  if (entry.stage === 'complete') return null;
  return (
    <span className={`stage ${entry.stage === 'failed' ? 'failed' : ''}`}>
      {isProcessing(entry) && <LoaderCircle className="spin" size={12} aria-hidden />}
      {stageName(entry)}
    </span>
  );
}
