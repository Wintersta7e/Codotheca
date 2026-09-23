import { configure } from '@testing-library/react';

// Testing Library's `findBy*` and `waitFor` give up after 1000 ms by default. A hosted Windows
// runner ran this suite roughly sixty times slower than a workstation, and a page test whose
// awaited element appears in 16 ms locally ran past that budget there. A wait still resolves the
// moment its element exists, so the budget only decides how long a real failure takes to report;
// it stays under Vitest's 5000 ms test timeout so that failure still prints its DOM.
configure({ asyncUtilTimeout: 4000 });
