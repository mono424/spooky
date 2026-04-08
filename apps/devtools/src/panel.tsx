import { render } from 'solid-js/web';
import { App } from './App';
// oxlint-disable-next-line no-unassigned-import
import './styles/devtools.css';

console.log('[DevTools Panel] Initializing...');

const root = document.getElementById('root');

if (!root) {
  throw new Error('Root element not found');
}

render(() => <App />, root);

console.log('[DevTools Panel] Initialized successfully');
