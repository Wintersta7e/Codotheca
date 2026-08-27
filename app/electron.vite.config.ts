import { resolve } from 'node:path';
import react from '@vitejs/plugin-react';
import { defineConfig, externalizeDepsPlugin } from 'electron-vite';
import type { Plugin } from 'vite';
import { CONTENT_SECURITY_POLICY, developmentContentSecurityPolicy } from './src/shared/csp';

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
    plugins: [externalizeDepsPlugin()],
    build: {
      outDir: 'out/main',
      lib: { entry: resolve(__dirname, 'src/main/index.ts') },
    },
  },
  preload: {
    plugins: [externalizeDepsPlugin()],
    build: {
      outDir: 'out/preload',
      lib: { entry: resolve(__dirname, 'src/preload/index.ts') },
      // sandbox: true loads the preload as a classic script. ESM would not run.
      rollupOptions: { output: { format: 'cjs', entryFileNames: 'index.js' } },
    },
  },
  renderer: {
    root: resolve(__dirname, 'src/renderer'),
    plugins: [react(), contentSecurityPolicyPlugin()],
    build: {
      outDir: resolve(__dirname, 'out/renderer'),
      emptyOutDir: true,
      // Vite inlines assets under 4 KB as data: URIs. A data: URI font would need
      // `font-src data:` and would therefore reopen the clause §8.7 closed. Nothing is
      // inlined, so `font-src 'self'` holds without a caveat.
      assetsInlineLimit: 0,
      rollupOptions: { input: resolve(__dirname, 'src/renderer/index.html') },
    },
  },
});
