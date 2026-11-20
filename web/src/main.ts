import { mount } from 'svelte';
import './style.css';
import { start } from './early';

const link = location.pathname.match(/^\/s\/([^/%]+)\/?$/)?.[1];
// As the pages ask for them; an id or token with an escaped character just isn't read early.
const entry = location.pathname.match(/^\/entries\/([^/%]+)\/?$/)?.[1];
if (link) start([`/s/${link}/entry`], { credentials: 'omit' });
else
  start([
    '/api/v1/me',
    ...(entry
      ? [`/api/v1/entries/${entry}`, `/api/v1/entries/${entry}/share`, '/api/v1/people']
      : []),
  ]);
// A share link loads only its own page: importing the app would run owner code (its sign-in
// check, its router's link handling) on a page opened without signing in.
const root = location.pathname.startsWith('/s/')
  ? import('./share-app.svelte')
  : import('./app.svelte');
root.then(({ default: App }) => mount(App, { target: document.getElementById('root')! }));
