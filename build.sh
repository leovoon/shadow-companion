#!/bin/bash
# Build Shadow Meter as a macOS .app bundle
set -e

cd "$(dirname "$0")"

APP_NAME="Shadow Meter"
BUNDLE_ID="com.shadow-companion.meter"
APP_DIR="dist/${APP_NAME}.app"

echo "→ Compiling with Perry..."
PERRY_RUNTIME_DIR=/tmp perry compile src/main.ts -o dist/shadow-meter

echo "→ Creating .app bundle..."
rm -rf "$APP_DIR"
mkdir -p "$APP_DIR/Contents/MacOS"
mkdir -p "$APP_DIR/Contents/Resources"

cp dist/shadow-meter "$APP_DIR/Contents/MacOS/shadow-meter"
chmod +x "$APP_DIR/Contents/MacOS/shadow-meter"

cat > "$APP_DIR/Contents/Info.plist" << 'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key>
    <string>en</string>
    <key>CFBundleExecutable</key>
    <string>shadow-meter</string>
    <key>CFBundleIdentifier</key>
    <string>com.shadow-companion.meter</string>
    <key>CFBundleName</key>
    <string>Shadow Meter</string>
    <key>CFBundleDisplayName</key>
    <string>Shadow Meter</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>1.0.0</string>
    <key>CFBundleVersion</key>
    <string>1</string>
    <key>LSMinimumSystemVersion</key>
    <string>13.0</string>
    <key>LSUIElement</key>
    <true/>
    <key>NSHighResolutionCapable</key>
    <true/>
</dict>
</plist>
EOF

echo "→ Done: $APP_DIR"
echo "  Run: open \"$APP_DIR\""
