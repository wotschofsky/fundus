import { Link, useLocation, useNavigate } from 'react-router-dom';
import { ArrowLeft } from 'lucide-react';

/** Back to the page you came from (search, a filtered list), or to Recordings from a fresh tab. */
export function BackLink() {
  const navigate = useNavigate();
  const inApp = useLocation().key !== 'default';
  return (
    <Link
      className="back"
      to="/"
      onClick={(event) => {
        if (!inApp) return;
        event.preventDefault();
        navigate(-1);
      }}
    >
      <ArrowLeft size={15} aria-hidden />
      {inApp ? 'Back' : 'Recordings'}
    </Link>
  );
}
