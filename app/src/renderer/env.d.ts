/// <reference types="vite/client" />
import type { CodothecaBridge } from '../shared/bridge';

declare global {
  interface Window {
    readonly codotheca: CodothecaBridge;
  }
}
