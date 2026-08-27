import type { ReactElement } from 'react';

export function App(): ReactElement {
  return (
    <main
      style={{
        display: 'grid',
        placeItems: 'center',
        height: '100vh',
        color: '#dde3e8',
        fontFamily: "'Barlow', system-ui, sans-serif",
        fontSize: '13px',
      }}
    >
      <p>Codotheca</p>
    </main>
  );
}
