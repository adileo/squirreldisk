import { OsType } from "@tauri-apps/plugin-os";

export {};
declare global {
  interface Window {
    OS_TYPE: OsType;
  }
}
