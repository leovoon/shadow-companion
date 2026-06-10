// Shadow Meter — Perry menubar app
// Shows daily Handy STT recording progress as a menubar icon

import { readFileSync, existsSync } from "fs"
import { join } from "path"
import { homedir } from "os"
import { exec } from "child_process"
import {
    App,
    trayCreate,
    traySetIcon,
    traySetTooltip,
    trayAttachMenu,
    menuCreate,
    menuAddItem,
    menuAddSeparator,
} from "perry/ui"

// ── config ────────────────────────────────────────────────────────

const HOME = homedir()
const PROGRESS_PATH = join(HOME, ".shadow-companion", "daily-progress.json")
const STATE_PATH = join(HOME, ".shadow-companion", "state.json")
const ICONS_DIR = join(HOME, ".shadow-companion", "perry-icons")

// ── state ─────────────────────────────────────────────────────────

let showVisual = true  // toggle between visual and text mode

// ── progress reader ───────────────────────────────────────────────

interface Progress {
    date: string
    actual_seconds: number
    target_seconds: number
    progress: number
}

function readProgress(): Progress | null {
    try {
        if (!existsSync(PROGRESS_PATH)) return null
        const raw = readFileSync(PROGRESS_PATH, "utf-8")
        return JSON.parse(raw) as Progress
    } catch {
        return null
    }
}

// ── icon selection ────────────────────────────────────────────────

function progressToSlices(progress: number): number {
    const clamped = Math.max(0, Math.min(1, progress))
    if (clamped === 0) return 0
    return Math.ceil(clamped * 5)
}

function iconExists(path: string): boolean {
    try { return existsSync(path) } catch { return false }
}

function getIconPath(progress: Progress): string {
    const slices = progressToSlices(progress.progress)
    const batteryIcon = join(ICONS_DIR, `battery-${slices}.png`)

    if (showVisual) {
        return batteryIcon
    }

    const actualMin = Math.floor(progress.actual_seconds / 60)
    const targetMin = Math.floor(progress.target_seconds / 60)
    const rounded = Math.min(180, Math.round(actualMin / 5) * 5)
    const textIcon = join(ICONS_DIR, `text-${rounded}-${targetMin}.png`)

    if (iconExists(textIcon)) {
        return textIcon
    }
    return batteryIcon
}

function getTooltip(progress: Progress): string {
    const actualMin = Math.floor(progress.actual_seconds / 60)
    const targetMin = Math.floor(progress.target_seconds / 60)
    return `${actualMin}/${targetMin} min — Shadow Meter`
}

// ── update ────────────────────────────────────────────────────────

function update() {
    const progress = readProgress()
    if (progress) {
        traySetIcon(tray, getIconPath(progress))
        traySetTooltip(tray, getTooltip(progress))
    } else {
        traySetIcon(tray, join(ICONS_DIR, "battery-0.png"))
        traySetTooltip(tray, "No data — run shadow.py first")
    }
    // Rebuild menu (toggle label changes)
    rebuildMenu()
}

// ── menu ──────────────────────────────────────────────────────────

let tray: number
let menu: number

function rebuildMenu() {
    menu = menuCreate()
    const modeLabel = showVisual ? "Switch to Text Mode" : "Switch to Battery Mode"
    menuAddItem(menu, modeLabel, () => {
        showVisual = !showVisual
        update()
    })
    menuAddSeparator(menu)
    menuAddItem(menu, "Open Config", () => {
        exec(`open "${STATE_PATH}"`)
    })
    menuAddItem(menu, "Open Progress File", () => {
        exec(`open "${PROGRESS_PATH}"`)
    })
    menuAddSeparator(menu)
    menuAddItem(menu, "Quit", () => {
        process.exit(0)
    })
    trayAttachMenu(tray, menu)
}

// ── initial icon ──────────────────────────────────────────────────

function initialIconPath(): string {
    const progress = readProgress()
    if (progress) {
        return getIconPath(progress)
    }
    return join(ICONS_DIR, "battery-0.png")
}

// ── tray setup ────────────────────────────────────────────────────

// Create tray BEFORE App() — required by Perry
// Pass real icon path (not "") to avoid the ● placeholder dot
tray = trayCreate(initialIconPath())

// Build initial menu
rebuildMenu()

// ── app ────────────────────────────────────────────────────────────

App({
    title: "Shadow Meter",
    width: 1,
    height: 1,
    frameless: true,
    activationPolicy: "accessory",
})

// First update + polling (15s — progress only changes on new recordings)
update()
setInterval(update, 15_000)
