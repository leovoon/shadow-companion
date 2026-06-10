/// <reference types="@raycast/api">

/* 🚧 🚧 🚧
 * This file is auto-generated from the extension's manifest.
 * Do not modify manually. Instead, update the `package.json` file.
 * 🚧 🚧 🚧 */

/* eslint-disable @typescript-eslint/ban-types */

type ExtensionPreferences = {}

/** Preferences accessible in all the extension's commands */
declare type Preferences = ExtensionPreferences

declare namespace Preferences {
  /** Preferences accessible in the `control` command */
  export type Control = ExtensionPreferences & {}
  /** Preferences accessible in the `switch-voice` command */
  export type SwitchVoice = ExtensionPreferences & {}
  /** Preferences accessible in the `switch-provider` command */
  export type SwitchProvider = ExtensionPreferences & {}
  /** Preferences accessible in the `adjust-speed` command */
  export type AdjustSpeed = ExtensionPreferences & {}
}

declare namespace Arguments {
  /** Arguments passed to the `control` command */
  export type Control = {}
  /** Arguments passed to the `switch-voice` command */
  export type SwitchVoice = {}
  /** Arguments passed to the `switch-provider` command */
  export type SwitchProvider = {}
  /** Arguments passed to the `adjust-speed` command */
  export type AdjustSpeed = {}
}

