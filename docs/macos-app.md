# macOS App (turm-macos)

## Overview

Swift/AppKit app using [SwiftTerm](https://github.com/migueldeicaza/SwiftTerm) for terminal rendering and PTY management. SwiftTerm's `LocalProcessTerminalView` handles the PTY internally — the same design choice as VTE on Linux: no custom PTY layer needed.

**Tech stack:** Swift 6, AppKit, SwiftTerm 1.11+, macOS 14+

**Build system:** Swift Package Manager (standalone, not in Cargo workspace)

---

## Architecture

### Linux vs macOS 비교

| 항목 | Linux | macOS |
|---|---|---|
| UI framework | GTK4 | AppKit (NSWindow/NSViewController) |
| Terminal widget | VTE4 (`vte4::Terminal`) | SwiftTerm (`LocalProcessTerminalView`) |
| PTY 관리 | VTE 내장 (`spawn_async`) | SwiftTerm 내장 (`startProcess`) |
| IPC | D-Bus + Unix socket | Unix socket (예정) |
| 메인 스레드 전달 | `glib::timeout_add_local` 폴링 | `DispatchQueue.main.async` |
| 설정 파싱 | `toml` crate | 직접 구현 (simple line parser) |
| 테마 | `turm-core/theme.rs` | `Theme.swift` (mirrors Rust struct) |

### 디렉토리 구조

```
turm-macos/
├── Package.swift                      # SwiftTerm 의존성 선언
├── run.sh                             # .app 번들 생성 후 실행
└── Sources/Turm/
    ├── TurmApp.swift                  # @main 진입점
    ├── AppDelegate.swift              # NSWindow 생성, 메뉴바 구성
    ├── TerminalViewController.swift   # SwiftTerm 래퍼, 쉘 실행, delegate
    ├── Config.swift                   # config.toml 파서
    └── Theme.swift                    # 10개 내장 테마 (RGBColor, TurmTheme)
```

---

## 빌드 및 실행

```bash
# 개발 중 빠른 테스트 (메뉴바 이름이 올바르지 않을 수 있음)
cd turm-macos
swift run

# 제대로 된 .app 번들로 실행 (권장)
./run.sh
# → .build/debug/Turm.app 생성 후 open으로 실행

# 빌드만
swift build
# → .build/debug/Turm 바이너리 생성
```

`run.sh`는 매번 `Turm.app/Contents/Info.plist`를 포함한 번들을 새로 만들어서 `open`으로 실행합니다. Info.plist가 있어야 Dock 아이콘, 메뉴바 앱 이름 등이 정상 표시됩니다.

---

## 파일별 구현 세부사항

### TurmApp.swift

`@main`으로 진입점 선언. `NSApplication.shared`를 직접 다루어 `AppDelegate`를 설정하고 `app.run()`으로 이벤트 루프 시작.

### AppDelegate.swift (`@MainActor`)

- `TurmConfig.load()` → `TurmTheme.byName()` 순서로 설정·테마 로드
- `NSWindow` 생성 (1200×800, titled/closable/resizable/miniaturizable)
- `TerminalViewController`를 `window.contentViewController`로 설정
- 메뉴바: App(종료), View(줌 인/아웃/리셋) 구성

### TerminalViewController.swift (`@MainActor`)

핵심 클래스. `loadView()`에서 `LocalProcessTerminalView`를 생성해 `self.view`로 설정.

**초기화 순서:**
```
loadView()
  └─ LocalProcessTerminalView 생성
  └─ configureColors() → nativeBackgroundColor, nativeForegroundColor, installColors()
  └─ configureFont()   → NSFont(name:size:) 또는 monospacedSystemFont 폴백
viewDidLoad()
  └─ startShell()
       └─ 현재 환경 상속 + TERM/COLORTERM/TURM_SOCKET 추가
       └─ tv.startProcess(executable:args:environment:execName:)
```

**환경변수 처리:**
```swift
var env = ProcessInfo.processInfo.environment.map { "\($0.key)=\($0.value)" }
env.append("TERM=xterm-256color")
env.append("COLORTERM=truecolor")
env.append("TURM_SOCKET=/tmp/turm-\(pid).sock")
```
`environment` 파라미터에 배열을 넘기면 부모 환경이 완전히 교체됩니다. 반드시 현재 환경을 상속해서 TERM 등 필수 변수가 빠지지 않도록 해야 합니다.

**LocalProcessTerminalViewDelegate 구현:**

| 메서드 | 처리 |
|---|---|
| `sizeChanged` | no-op (SwiftTerm이 내부 처리) |
| `setTerminalTitle` | `DispatchQueue.main` → `window.title` 업데이트 |
| `processTerminated` | `DispatchQueue.main` → `window.close()` |
| `hostCurrentDirectoryUpdate` | no-op (향후 OSC 7 이벤트 연동 예정) |

delegate 메서드는 `nonisolated`로 선언하고 UI 작업은 `Task { @MainActor in ... }` 패턴으로 메인 스레드에 전달합니다.

**줌:**
SwiftTerm에 내장 줌 기능 없음. `currentFontSize`를 추적하며 `tv.font`를 새 크기의 NSFont로 교체하는 방식으로 구현.

### Config.swift

`~/.config/turm/config.toml`을 직접 파싱. turm-core의 설정 파일과 동일한 포맷.

파싱 규칙:
- `[section]` 헤더로 섹션 구분
- `key = "value"` 에서 따옴표 제거
- `# 인라인 코멘트` 제거
- 파일 없으면 기본값 사용 (`$SHELL`, font 14pt, catppuccin-mocha)

현재 읽는 필드: `terminal.shell`, `terminal.font_family`, `terminal.font_size`, `theme.name`

### Theme.swift

`RGBColor(hex:)` — hex 문자열 → `(r, g, b: UInt8)` 변환.

SwiftTerm에 넘길 때 8비트 → 16비트 변환:
```swift
SwiftTerm.Color(
    red:   UInt16(c.r) * 257,
    green: UInt16(c.g) * 257,
    blue:  UInt16(c.b) * 257
)
```
`* 257`(= `0x101`)로 곱하면 0 → 0, 255 → 65535로 정확히 매핑됩니다.

10개 내장 테마는 `turm-core/theme.rs`와 동일한 팔레트값을 사용합니다.

---

## 소켓 IPC (예정)

`turmctl`이 macOS 앱을 제어하려면 Swift에서 Unix 소켓 서버를 구현해야 합니다.

**계획된 아키텍처:**
```
turmctl ──Unix socket──► SocketServer (Swift, background thread)
                                │
                    DispatchQueue.main.async
                                │
                       TerminalViewController
                                │
                       Response → socket thread → turmctl
```

Linux(`socket.rs`)와 달리 50ms 폴링 없이 `DispatchQueue.main.async`로 직접 메인 스레드에 디스패치합니다.

프로토콜은 turm-core `protocol.rs`와 동일: newline-delimited JSON (`Request` → `Response`).

---

## 구현 로드맵

### Phase 1: MVP ✅
- SwiftTerm 통합, 쉘 실행, 테마, 폰트 설정, 줌, 창 제목 업데이트, 종료 처리

### Phase 2: 소켓 IPC
- Unix 소켓 서버 (`SocketServer.swift`)
- `system.ping`, `terminal.exec`, `terminal.feed`, `terminal.read`, `terminal.state`

### Phase 3: 탭
- `NSTabViewController` 또는 커스텀 탭바
- 탭 추가/닫기/전환 (Cmd+T / Cmd+W / Cmd+1-9)

### Phase 4: 분할 창
- 수평/수직 분할 (`NSSplitView` 기반)
- Linux의 `SplitNode` 트리와 동일한 구조

### Phase 5: 배경 이미지
- SwiftTerm은 배경 투명도를 직접 지원하지 않음
- 방법: `LocalProcessTerminalView`를 `NSView` 컨테이너에 올리고 컨테이너 배경에 이미지 렌더링 + `layer.backgroundColor = .clear`로 SwiftTerm 배경 투명 처리

### Phase 6: 인터미널 검색
- SwiftTerm의 `TerminalViewSearch` API (`search(_:)`, `searchNext()`, `searchPrevious()`) 활용
- 검색바 NSView 오버레이

---

## 알려진 주의사항

- **`fullSizeContentView` 금지**: 이 styleMask를 쓰면 콘텐츠가 타이틀바 아래까지 확장되어 SwiftTerm이 행 수를 잘못 계산하고 커서 위치가 한 행 어긋납니다.
- **환경변수 상속 필수**: `startProcess(environment:)`에 배열을 넘기면 부모 환경 완전 교체. `TERM` 없이 쉘 실행 시 오류 발생.
- **`hostCurrentDirectoryUpdate` 구현 필수**: `LocalProcessTerminalViewDelegate`에 선택적 메서드가 아니므로 stub이라도 있어야 컴파일됩니다.
