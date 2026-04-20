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
| IPC | D-Bus + Unix socket | Unix socket |
| 메인 스레드 전달 | `glib::timeout_add_local` 폴링 | `DispatchQueue.main.sync` (동기) |
| 설정 파싱 | `toml` crate | 직접 구현 (simple line parser) |
| 테마 | `turm-core/theme.rs` | `Theme.swift` (mirrors Rust struct) |
| 분할 창 | GTK Paned | `NSSplitView` + `EqualSplitView` |

### 디렉토리 구조

```
turm-macos/
├── Package.swift                      # SwiftTerm 의존성 선언
├── run.sh                             # .app 번들 생성 후 실행
└── Sources/Turm/
    ├── TurmApp.swift                  # @main 진입점
    ├── AppDelegate.swift              # NSWindow 생성, 메뉴바, 소켓 커맨드 라우팅
    ├── TabViewController.swift        # 탭 목록 관리, PaneManager 배열
    ├── TabBarView.swift               # 커스텀 탭바 UI + 패널 추가 팝오버
    ├── PaneManager.swift              # 단일 탭의 split-pane 트리 관리
    ├── SplitNode.swift                # N-ary 분할 트리 데이터 구조
    ├── TurmPanel.swift                # 모든 패널 타입 공통 프로토콜
    ├── TerminalViewController.swift   # SwiftTerm 래퍼, 쉘 실행, delegate
    ├── WebViewController.swift        # WKWebView 래퍼, TurmPanel 구현
    ├── EventBus.swift                 # 이벤트 브로드캐스트 허브 + 채널
    ├── SocketServer.swift             # POSIX Unix socket 서버 (async handler)
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
```

`run.sh`는 매번 `Turm.app/Contents/Info.plist`를 포함한 번들을 새로 만들어서 `open`으로 실행합니다. Info.plist가 있어야 Dock 아이콘, 메뉴바 앱 이름 등이 정상 표시됩니다.

---

## 파일별 구현 세부사항

### TurmApp.swift

`@main`으로 진입점 선언. `NSApplication.shared`를 직접 다루어 `AppDelegate`를 설정하고 `app.run()`으로 이벤트 루프 시작.

### AppDelegate.swift (`@MainActor`)

- `TurmConfig.load()` → `TurmTheme.byName()` 순서로 설정·테마 로드
- `NSWindow` 생성 (1200×800, titled/closable/resizable/miniaturizable)
- `TabViewController`를 `window.contentViewController`로 설정
- 메뉴바: App(종료), Shell(탭/분할/전환), Find(검색), View(줌) 구성
- 소켓 서버 시작 후 `handleCommand(method:params:)`로 모든 커맨드 라우팅

**Find 메뉴 (in-terminal search):**
`performFindPanelAction(_:)` 메서드를 AppDelegate에 직접 선언하고, 활성 터미널 뷰에 동일한 셀렉터로 포워딩합니다. SwiftTerm의 `MacTerminalView`가 `performFindPanelAction(_:)`을 구현하고 있어서 Cmd+F / Cmd+G / Cmd+Shift+G가 SwiftTerm 내장 검색바를 트리거합니다. 검색바에는 case-sensitive, regex, whole-word 옵션이 포함됩니다.

### TabViewController.swift (`@MainActor`)

탭 목록을 `[PaneManager]`로 관리. `contentArea`에 현재 탭의 `containerView`를 embed합니다.

**주요 흐름:**
```
newTab() / newWebViewTab(url:)
  └─ PaneManager 생성 → onLastPaneClosed / onActivePaneChanged 연결
  └─ NotificationCenter로 terminalTitleChanged 구독
  └─ eventBus 전파
  └─ tab.opened 이벤트 발행
  └─ switchTab(to: last)

switchTab(to:)
  └─ 이전 containerView removeFromSuperview
  └─ 새 containerView를 contentArea에 fill constraints로 embed
  └─ layoutSubtreeIfNeeded() → startIfNeeded() → makeFirstResponder()
```

**주요 프로퍼티:**
- `activeTerminal: TerminalViewController?` — 활성 터미널 패널
- `activeWebView: WebViewController?` — 활성 웹뷰 패널
- `eventBus: EventBus?` — didSet 시 모든 PaneManager에 전파

**소켓 커맨드 메서드:** `tabList()`, `tabInfo()`, `renameTab(at:title:)`, `sessionList()`, `sessionInfo(index:)`, `execCommand(_:)`, `feedText(_:)`, `terminalState()`, `readScreen()`

### TabBarView.swift

NSView 기반 커스텀 탭바. TurmTheme 색상을 사용합니다.

- 각 탭: 제목 label + × 버튼, hover 시 배경색 전환
- 오른쪽 끝: + 버튼 → `AddPanelPopoverController` 팝오버 표시
- `onSelectTab`, `onCloseTab`, `onNewPanel` 클로저로 이벤트 전달
- `TabBarView.height = 36` (상수)

**`AddPanelPopoverController`:**
Terminal / Browser 행 × Tab / Split Horizontal / Split Vertical 버튼(SF Symbol 아이콘). `onNewPanel: ((AddPanelType, AddPanelMode) -> Void)?` 클로저로 결과 전달. Linux의 GTK Popover와 동일한 UX.

### PaneManager.swift (`@MainActor`)

단일 탭의 분할 창 트리를 관리합니다.

**핵심 설계:**
- `containerView` (NSView): TabViewController가 한 번만 embed, 이후 내부만 rebuild
- `root: SplitNode`: 현재 분할 상태의 N-ary 트리
- `activePane: any TurmPanel`: 현재 포커스된 패널 (터미널 또는 웹뷰)
- `eventBus: EventBus?`: didSet 시 모든 패널에 `assignEventBus(to:)` 전파
- split/close 때마다 `rebuildViewHierarchy()` 호출 → fresh `EqualSplitView` 트리 생성

**`EqualSplitView` (private):**
`NSSplitView` + `NSSplitViewDelegate`를 결합한 내부 클래스.
`splitView(_:resizeSubviewsWithOldSize:)` delegate에서 첫 번째 호출 시 모든 subview를 균등 분배하고 `initialSizeSet = true`로 잠금. 이후 호출은 `adjustSubviews()`(기본 동작)에 위임해 사용자 드래그가 동작합니다.

`layout()`이 아닌 delegate를 쓰는 이유: NSSplitView는 `resizeSubviews`(→ delegate)로 subview frame을 확정한 뒤 `layout()`을 호출합니다. `layout()` 시점에는 이미 잘못된 frame이 커밋된 상태이므로 `setPosition`을 거기서 불러도 신뢰할 수 없습니다. delegate를 사용하면 NSSplitView가 "지금 subview frame 정해"라고 위임하는 바로 그 순간에 개입할 수 있습니다.

**포커스 감지:**
SwiftTerm의 `MacTerminalView.becomeFirstResponder`는 `public`이지만 `open`이 아니라서 외부 모듈에서 override 불가. `NSEvent.addLocalMonitorForEvents(matching: .leftMouseDown)`으로 클릭 위치를 확인해 포커스 전환합니다.

**터미널 종료 연결:**
`wirePanel(_:)`에서 `TerminalViewController`로 캐스트해 `onProcessTerminated` 클로저 등록. 활성 pane이면 `closeActive()`, 비활성이면 트리에서 제거 후 rebuild.

### SplitNode.swift

N-ary 재귀 트리 enum:

```swift
indirect enum SplitNode {
    case leaf(any TurmPanel)                    // 터미널 또는 웹뷰
    case branch(SplitOrientation, [SplitNode])  // N개 자식 가능
}
```

동일성 비교는 `===` 대신 `ObjectIdentifier`를 사용합니다 (`any TurmPanel`은 `Equatable`이 아님).

**`splitting(_:with:orientation:)` — 항상 계층형 분할:**
focused leaf를 항상 새 2-child branch로 교체합니다. 같은 방향의 부모 branch에 flat하게 추가하지 않습니다.

```
[A] → split →
  branch(H, [leaf(A), leaf(B)])
  A=50%, B=50%

focus A → split →
  branch(H, [branch(H, [leaf(A), leaf(C)]), leaf(B)])
  A=25%, C=25%, B=50%  ← B 크기 변화 없음
```

**`removing(_:)` — collapse:**
제거 후 자식이 1개만 남으면 branch를 collapse해 단일 leaf로 승격.

### TerminalViewController.swift (`@MainActor`)

**`TurmTerminalView` (private subclass):**
SwiftTerm 버그 우회를 위한 래퍼. `installExitMonitor()`에서 별도 `DispatchSource.makeProcessSource`를 설치. 자세한 내용은 troubleshooting.md 참조.

**`startIfNeeded()`:**
Shell은 뷰가 계층에 추가되고 `layoutSubtreeIfNeeded()` 이후에만 시작. frame 없이 `startProcess`를 호출하면 SwiftTerm이 행/열 수를 0으로 계산합니다.

**환경변수:**
```swift
var env = ProcessInfo.processInfo.environment.map { "\($0.key)=\($0.value)" }
env.append("TERM=xterm-256color")
env.append("COLORTERM=truecolor")
env.append("TURM_SOCKET=/tmp/turm-\(pid).sock")
```
`startProcess(environment:)`에 배열을 넘기면 부모 환경이 완전히 교체되므로 반드시 현재 환경을 상속해야 합니다.

**소켓 커맨드 메서드:**
- `execCommand(_:)` — 명령어 + 줄바꿈 PTY 전송
- `feedText(_:)` — 원시 텍스트 PTY 전송
- `terminalState()` — cols/rows/cursor/title
- `readScreen()` — 현재 화면 텍스트 + 커서 위치
- `history(lines:)` — 스크롤백 N줄 (SwiftTerm 음수 row 인덱스 활용)
- `context(historyLines:)` — state + screen + history 합산
- `setCustomTitle(_:)` — 커스텀 탭 제목 (자동 업데이트 억제)

**`setTerminalTitle` delegate:**
```swift
nonisolated func setTerminalTitle(...) {
    Task { @MainActor in
        guard self.customTitle == nil else { return }  // 커스텀 제목이 있으면 무시
        self.currentTitle = title.isEmpty ? "Terminal" : title
        ...
        eventBus?.broadcast(event: "panel.title_changed", ...)
    }
}
```

**`hostCurrentDirectoryUpdate` delegate:**
OSC 7 URI(`file://hostname/path`)에서 `URL(string:).path`로 hostname을 제거해 `/Users/...` 형태의 순수 경로만 추출 후 `terminal.cwd_changed` 이벤트 발행.

**`processTerminated` delegate:**
`panel.exited` 이벤트 발행 후 `onProcessTerminated` 클로저 호출.

### TurmPanel.swift

모든 패널 타입의 공통 인터페이스:

```swift
@MainActor
protocol TurmPanel: AnyObject {
    var view: NSView { get }
    var currentTitle: String { get }
    func startIfNeeded()
    func applyBackground(path: String, tint: Double)
    func clearBackground()
    func setTint(_ alpha: Double)
    func removeFromParent()
}
```

`TerminalViewController`와 `WebViewController` 모두 이 프로토콜을 구현. `SplitNode`와 `PaneManager`는 `any TurmPanel`로 동작해서 터미널과 웹뷰를 같은 분할 트리에 섞어 쓸 수 있음.

### WebViewController.swift (`@MainActor`)

`WKWebView` 래퍼. `TurmPanel` 프로토콜 구현.

- `startIfNeeded()` — 초기 URL 로드 (없으면 blank page)
- `navigate(to:)` — URL 탐색 (scheme 없으면 `https://` 자동 추가)
- `goBack()` / `goForward()` / `reload()` — 네비게이션
- `executeJS(_:completion:)` — JS 평가 (비동기, WKWebView 콜백)
- `getContent(completion:)` — `document.documentElement.outerHTML` 반환
- `toggleDevTools()` — `developerExtrasEnabled` 토글 (Safari Web Inspector)
- `WKNavigationDelegate.webView(_:didFinish:)` — 탭 제목 업데이트 → `terminalTitleChanged` 알림 발행

**배경 이미지:** `applyBackground`, `clearBackground`, `setTint` 모두 no-op (WebView는 자체 렌더링).

### EventBus.swift

이벤트 브로드캐스트 허브. Linux의 `Arc<Mutex<Vec<Sender<String>>>>`와 동일한 역할.

```swift
final class EventBus: @unchecked Sendable {
    func subscribe() -> EventChannel       // 구독자 채널 반환
    func broadcast(event: String, data: [String: Any] = [:])
}

final class EventChannel: @unchecked Sendable {
    func send(_ event: String) -> Bool     // false = 채널 닫힘
    func receive() -> String?              // 이벤트 또는 close까지 블록
    func close()
}
```

`EventChannel`은 `NSLock` + `DispatchSemaphore`로 thread-safe 버퍼드 FIFO 구현. `SocketServer`의 `streamEvents(fd:)`가 `channel.receive()`에서 블록하며 이벤트를 클라이언트로 스트리밍.

**이벤트 형식:** `{"event":"<type>","data":{...}}\n`

| 이벤트 | data | 발생 시점 |
|--------|------|-----------|
| `panel.focused` | `{panel_id}` | 클릭으로 pane 포커스 |
| `panel.title_changed` | `{panel_id, title}` | 터미널/웹뷰 제목 변경 |
| `panel.exited` | `{panel_id}` | 쉘 프로세스 종료 |
| `tab.opened` | `{index, panel_id}` | 탭 생성 |
| `tab.closed` | `{index}` | 탭 닫힘 |
| `terminal.cwd_changed` | `{panel_id, cwd}` | OSC 7 CWD 변경 |
| `terminal.shell_precmd` | `{panel_id}` | 쉘 프롬프트 준비 |
| `terminal.shell_preexec` | `{panel_id}` | 명령 실행 직전 |
| `webview.loaded` | `{panel_id}` | 페이지 로드 완료 |
| `webview.title_changed` | `{panel_id, title}` | 웹뷰 제목 변경 |
| `webview.navigated` | `{panel_id, url}` | 웹뷰 URL 변경 |

### SocketServer.swift (비동기 핸들러)

```swift
var commandHandler: ((_ method: String, _ params: [String: Any], _ completion: @escaping (Any?) -> Void) -> Void)?
```

기존 동기 패턴에서 completion 기반으로 변경. 소켓 스레드는 `DispatchSemaphore`로 블록하고, 메인 스레드에서 핸들러가 completion을 호출하면 unblock. `webview.execute_js`처럼 WKWebView 콜백 이후에 응답해야 하는 커맨드를 지원.

**`ResultBox`**: `@unchecked Sendable` 클래스로 메인 액터 → 소켓 스레드 간 결과 전달. 세마포어가 직렬화를 보장하므로 안전.

### Config.swift

`~/.config/turm/config.toml`을 직접 파싱.

**TOML 구조:**
```toml
[terminal]
shell = "/bin/zsh"
font_family = "JetBrains Mono"
font_size = 13

[theme]
name = "catppuccin-mocha"
```

파싱 규칙: `[section]` 헤더로 섹션 구분, 따옴표/인라인 주석 제거.

**기본 폰트:** `"JetBrains Mono"` — macOS에 기본 설치된 NSFont family 이름. Nerd Font 변형(`JetBrainsMono Nerd Font Mono`)은 별도 설치 필요하므로 기본값으로 쓰지 않습니다.

### Theme.swift

`RGBColor(hex:)` — hex 문자열 → `(r, g, b: UInt8)` 변환.

SwiftTerm에 넘길 때 8비트 → 16비트 변환:
```swift
SwiftTerm.Color(red: UInt16(c.r) * 257, green: UInt16(c.g) * 257, blue: UInt16(c.b) * 257)
```
`* 257`(= `0x101`)로 곱하면 0 → 0, 255 → 65535로 정확히 매핑됩니다.

---

## 소켓 IPC

**아키텍처:**
```
turmctl ──Unix socket──► SocketServer (background thread)
                                │
                    DispatchQueue.main.sync
                                │
                       AppDelegate.handleCommand()
                                │
                    TabViewController / TerminalViewController
                                │
                       Response → socket thread → turmctl
```

프로토콜: turm-core `protocol.rs`와 동일한 newline-delimited JSON.

**지원 커맨드:**

| 커맨드 | 파라미터 | 동작 |
|---|---|---|
| `system.ping` | — | `{"status":"ok"}` |
| `event.subscribe` | — | 이벤트 스트림 구독 (long-lived 연결) |
| `terminal.exec` | `command` | 명령어 + 줄바꿈 PTY 전송 |
| `terminal.feed` | `text` | 원시 텍스트 PTY 전송 |
| `terminal.state` | — | cols/rows/cursor/title |
| `terminal.read` | — | 현재 화면 텍스트 + 커서 |
| `terminal.history` | `lines` (기본 100) | 스크롤백 텍스트 |
| `terminal.context` | `history_lines` (기본 50) | state + screen + history |
| `terminal.shell_precmd` | `panel_id` (선택) | shell_precmd 이벤트 발행 |
| `terminal.shell_preexec` | `panel_id` (선택) | shell_preexec 이벤트 발행 |
| `tab.new` | — | 새 터미널 탭 생성 |
| `tab.close` | — | 활성 pane 닫기 |
| `tab.switch` | `index` | 탭 전환 |
| `tab.list` | — | 탭 목록 |
| `tab.info` | — | 탭 목록 + pane 수 |
| `tab.rename` | `index`, `title` | 탭 이름 변경 (자동 제목 억제) |
| `split.horizontal` | — | 터미널 좌우 분할 |
| `split.vertical` | — | 터미널 상하 분할 |
| `session.list` | — | `tab.list`와 동일 |
| `session.info` | `index` | 특정 탭의 상세 정보 |
| `background.set` | `path`, `tint` (기본 0.6) | 배경 이미지 설정 |
| `background.set_tint` | `tint` | 틴트 알파값 변경 |
| `background.clear` | — | 배경 이미지 제거 |
| `agent.approve` | `message`, `title?`, `actions?` | NSAlert 모달 표시, 비동기 응답 |
| `webview.open` | `url?`, `mode?` (tab/split_h/split_v) | 웹뷰 패널 생성 |
| `webview.navigate` | `url` | 활성 웹뷰 URL 이동 |
| `webview.back` | — | 뒤로 |
| `webview.forward` | — | 앞으로 |
| `webview.reload` | — | 새로고침 |
| `webview.execute_js` | `script` | JS 평가 (비동기 응답) |
| `webview.get_content` | — | 페이지 HTML 반환 (비동기) |
| `webview.devtools` | — | Safari Web Inspector 토글 |
| `webview.state` | — | url/title/can_go_back/forward/is_loading |

---

## Linux 대비 미구현 기능 (포팅 예정)

### ~~Phase 2: WebView Panel~~ ✅ 구현 완료

`WebViewController.swift` + `TurmPanel.swift` 참조.

### ~~Phase 3: AI Agent & Shell Integration~~ ✅ 구현 완료

`EventBus.swift`, `SocketServer.swift` 참조. 단, 아래 항목은 SwiftTerm 제한으로 미구현:
- **`terminal.output` 이벤트** — SwiftTerm의 `feed(byteArray:)`가 extension에 선언되어 외부 모듈에서 override 불가. PTY 출력 인터셉트 방법 없음.
- **OSC 9/777 `terminal.notification` 이벤트** — SwiftTerm에 별도 delegate 없음.
- **Shell integration via OSC 133** — 위와 동일한 이유. 대신 `terminal.shell_precmd` / `terminal.shell_preexec`를 소켓 커맨드로 구현 (쉘 스크립트에서 직접 호출).

### Phase 4: Tab Bar & UX Polish
- ~~**Tab bar toggle** — 아이콘만 보이는 collapsed 모드 (Cmd+Shift+B), `tabs.toggle_bar` 소켓~~ ✅ 구현 완료
- **Double-click rename** — 탭 라벨 더블클릭으로 인라인 편집
- **Pane focus navigation** — 키보드로 다음/이전 pane 포커스 이동
- **Background random rotation** — `background.next` 소켓 커맨드, `[background] directory` 설정
- **Config hot-reload** — 파일 변경 감지 후 테마/설정 즉시 반영

### Phase 5: Distribution & Ecosystem
- Session persistence / restore
- Clipboard integration (OSC 52)
- URL detection + click-to-open
- Plugin system
- Status bar

---

## 알려진 주의사항

- **`fullSizeContentView` 금지**: 콘텐츠가 타이틀바 아래까지 확장되어 SwiftTerm이 행 수를 잘못 계산하고 커서 위치가 어긋납니다.
- **환경변수 상속 필수**: `startProcess(environment:)`에 배열을 넘기면 부모 환경 완전 교체. `TERM` 없이 쉘 실행 시 오류 발생.
- **`hostCurrentDirectoryUpdate` 구현 필수**: `LocalProcessTerminalViewDelegate`에 필수 메서드이므로 stub이라도 있어야 컴파일됩니다.
- **`startShellIfNeeded()` 타이밍**: `layoutSubtreeIfNeeded()` 이후에 호출해야 SwiftTerm이 올바른 cols/rows를 계산합니다.
- **`NSSplitView` subview layout**: NSSplitView의 직접 자식은 `translatesAutoresizingMaskIntoConstraints = true` + `autoresizingMask = [.width, .height]`를 써야 합니다. Auto Layout을 사용하면 NSSplitView와 충돌합니다.
- **SwiftTerm `processTerminated` 미호출 버그**: troubleshooting.md 참조.
