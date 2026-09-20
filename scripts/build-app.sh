#!/bin/bash
# scripts/build-app.sh — automaton .app 번들 생성
set -euo pipefail
REPO=$(cd "$(dirname "$0")/.." && pwd)
APP_NAME="Automaton"
BUNDLE="$REPO/build/$APP_NAME.app"

# 1. Swift 빌드
cd "$REPO/apps/Automaton" && swift build -c release

# 2. 번들 구조 생성
rm -rf "$BUNDLE"
mkdir -p "$BUNDLE/Contents/MacOS"
mkdir -p "$BUNDLE/Contents/Resources"

# 3. 실행 파일 복사
cp "$REPO/apps/Automaton/.build/release/Automaton" "$BUNDLE/Contents/MacOS/$APP_NAME"

# 4. Info.plist 생성 (LSUIElement=true로 메뉴바 전용)
cat > "$BUNDLE/Contents/Info.plist" << 'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key>
    <string>Automaton</string>
    <key>CFBundleIdentifier</key>
    <string>com.themagictower.automaton</string>
    <key>CFBundleName</key>
    <string>Automaton</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>0.1.0</string>
    <key>LSUIElement</key>
    <true/>
    <key>NSMicrophoneUsageDescription</key>
    <string>음성 명령 인식을 위해 마이크 접근이 필요합니다.</string>
</dict>
</plist>
PLIST

# 5. 아이콘 (기본 시스템 아이콘 사용 — 커스텀 아이콘은 후속)
# TODO: 커스텀 Brass & Glass 아이콘 생성

echo "✅ $BUNDLE 생성 완료"
echo "실행: open $BUNDLE"
