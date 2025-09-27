import { OsType } from "@tauri-apps/plugin-os";

export {};
declare global {
  interface Window {
    Headway: any;
    OS_TYPE: OsType;
  }
}
