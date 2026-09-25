import { resolve } from 'node:path';
import react from '@vitejs/plugin-react';
import { defineConfig } from 'electron-vite';
import type { Plugin } from 'vite';
import { CONTENT_SECURITY_POLICY, developmentContentSecurityPolicy } from './src/shared/csp';

/**
 * [p2] §25.5's four libraries, in one chunk with a name.
 *
 * One chunk rather than four, because the panel enters the whole stack through **one** dynamic
 * `import()` and `scripts/check-bundle.mjs` would otherwise have four names to chase — and the
 * names come from the module both this config and that gate read, so the set that goes **in** and
 * the set the gate keeps **out of first paint** cannot drift apart.
 */
import { MARKUP_PACKAGES, README_MARKUP_CHUNK } from '../scripts/lib/markup-chunk.mjs';

/** Which `node_modules` package a module id belongs to, or `null` for our own source. */
function packageOf(id: string): string | null {
  const parts = id.replace(/\\/g, '/').split('/node_modules/');
  const tail = parts.at(-1);
  if (parts.length < 2 || tail === undefined) return null;
  const segments = tail.split('/');
  const [first = '', second = ''] = segments;
  return first.startsWith('@') ? `${first}/${second}` : first;
}

/**
 * Writes `app/out/renderer/.chunk-map.json`: every emitted chunk, whether it is the entry, and the
 * `node_modules` packages among its module ids.
 *
 * It exists because a post-build gate reading concatenated `.js` text cannot tell which chunk a
 * module landed in, and guessing from minified text is a guess. The build already knows.
 */
function chunkMapPlugin(): Plugin {
  return {
    name: 'codotheca:chunk-map',
    generateBundle(_options, bundle) {
      const chunks = Object.values(bundle)
        .filter((output) => output.type === 'chunk')
        .map((chunk) => ({
          fileName: chunk.fileName,
          name: chunk.name,
          isEntry: chunk.isEntry,
          // **Static** imports and dynamic ones, kept apart. Which chunk a package landed in
          // says nothing on its own about whether first paint loads it: a static import from
          // the entry pulls the whole chunk in eagerly, and only this distinction can see that.
          imports: [...chunk.imports],
          dynamicImports: [...chunk.dynamicImports],
          packages: [
            ...new Set(
              Object.keys(chunk.modules)
                .map(packageOf)
                .filter((name): name is string => name !== null),
            ),
          ].sort(),
        }));
      this.emitFile({
        type: 'asset',
        fileName: '.chunk-map.json',
        source: `${JSON.stringify({ chunks }, null, 2)}\n`,
      });
    },
  };
}

function contentSecurityPolicyPlugin(): Plugin {
  return {
    name: 'codotheca:content-security-policy',
    transformIndexHtml: {
      order: 'pre',
      handler(html, ctx) {
        const port = ctx.server?.config.server.port ?? 5173;
        const policy =
          ctx.server === undefined
            ? CONTENT_SECURITY_POLICY
            : developmentContentSecurityPolicy(`http://localhost:${String(port)}`);
        return {
          html,
          tags: [
            {
              tag: 'meta',
              attrs: { 'http-equiv': 'Content-Security-Policy', content: policy },
              injectTo: 'head-prepend',
            },
          ],
        };
      },
    },
  };
}

export default defineConfig({
  main: {
    build: {
      externalizeDeps: true,
      outDir: 'out/main',
      lib: { entry: resolve(__dirname, 'src/main/index.ts') },
    },
  },
  preload: {
    build: {
      externalizeDeps: true,
      outDir: 'out/preload',
      lib: { entry: resolve(__dirname, 'src/preload/index.ts') },
      // sandbox: true loads the preload as a classic script. ESM would not run.
      rollupOptions: { output: { format: 'cjs', entryFileNames: 'index.js' } },
    },
  },
  renderer: {
    root: resolve(__dirname, 'src/renderer'),
    plugins: [react(), contentSecurityPolicyPlugin(), chunkMapPlugin()],
    build: {
      outDir: resolve(__dirname, 'out/renderer'),
      emptyOutDir: true,
      // Vite inlines assets under 4 KB as data: URIs. A data: URI font would need
      // `font-src data:` and would therefore reopen the clause §8.7 closed. Nothing is
      // inlined, so `font-src 'self'` holds without a caveat.
      assetsInlineLimit: 0,
      rollupOptions: {
        input: resolve(__dirname, 'src/renderer/index.html'),
        output: {
          manualChunks(id: string) {
            const owner = packageOf(id);
            return owner !== null && MARKUP_PACKAGES.includes(owner)
              ? README_MARKUP_CHUNK
              : undefined;
          },
        },
      },
    },
  },
});
