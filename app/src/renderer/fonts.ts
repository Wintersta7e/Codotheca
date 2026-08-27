/**
 * The three families, self-hosted (§8.7).
 *
 * The prototype links a remote font host with a preconnect to its asset origin. That must
 * not survive into the app: a font request is a request, and the footer's
 * `NO ACCOUNT · NO TELEMETRY` would be false. These imports make Vite emit the WOFF2 files as
 * bundle assets under the app's own origin, which is what `font-src 'self'` requires.
 *
 * Only the weights §8.7 names are imported. An unused weight is bytes in the installer and a
 * face a designer can reach for that the contract does not cover.
 */

// Display — headings, numbers, control labels, rank glyphs.
import '@fontsource/rajdhani/latin-500.css';
import '@fontsource/rajdhani/latin-600.css';
import '@fontsource/rajdhani/latin-700.css';

// Body — descriptions, prose, card copy.
import '@fontsource/barlow/latin-400.css';
import '@fontsource/barlow/latin-500.css';
import '@fontsource/barlow/latin-600.css';

// Mono — every number, path, label, keyboard hint.
import '@fontsource/jetbrains-mono/latin-400.css';
import '@fontsource/jetbrains-mono/latin-500.css';
import '@fontsource/jetbrains-mono/latin-700.css';
