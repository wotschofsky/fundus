import { mount } from 'svelte';
import './style.css';

// A share link loads only its own page: importing the app would run owner code (its sign-in
// check, its router's link handling) on a page opened without signing in.
const root = location.pathname.startsWith('/s/')
  ? import('./share-app.svelte')
  : import('./app.svelte');
root.then(({ default: App }) => mount(App, { target: document.getElementById('root')! }));
