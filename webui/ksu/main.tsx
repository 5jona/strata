import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { KsuStrataApp } from '../src/ksu/KsuStrataApp.tsx';
import '../src/styles.css';

const rootEl = document.getElementById('root');
if (rootEl) {
  createRoot(rootEl).render(
    <StrictMode>
      <KsuStrataApp />
    </StrictMode>,
  );
}
