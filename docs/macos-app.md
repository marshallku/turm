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

### Dev 루프 (debug bundle, 임시)

```bash
cd turm-macos
swift run                # 빠른 테스트 — 메뉴바 이름 등이 올바르지 않을 수 있음

./run.sh                 # 권장: .build/debug/Turm.app 새로 만들고 open으로 실행
swift build              # 빌드만
```

`run.sh`는 매번 `Turm.app/Contents/Info.plist`를 포함한 번들을 새로 만들어서 `open`으로 실행합니다. Info.plist가 있어야 Dock 아이콘, 메뉴바 앱 이름 등이 정상 표시됩니다.

### 영구 설치 (`/Applications`)

```bash
./scripts/install-macos.sh             # ~/Applications + ~/.cargo/bin/turmctl (no sudo)
./scripts/install-macos.sh --system    # /Applications + ~/.cargo/bin/turmctl (sudo for /Applications)
./scripts/install-macos.sh --launch    # 설치 후 바로 open
./scripts/install-macos.sh --no-build  # 이미 .build/release/Turm 있을 때
./scripts/install-macos.sh --no-turmctl # turmctl 재설치 스킵
```

설치 동작:
1. `cargo build --release -p turm-ffi` → `target/release/libturm_ffi.a` (Rust staticlib for Swift FFI)
2. `swift build -c release` → `turm-macos/.build/release/Turm` (links libturm_ffi.a)
3. `pkill -x Turm`로 실행 중 인스턴스 종료 (macOS는 실행 중 .app의 exec를 lock)
4. tmp 디렉토리에 `.app` staging → `mv`로 atomic 설치 (실패해도 dest가 깨지지 않음)
5. `cargo install --path turm-cli` → `~/.cargo/bin/turmctl`

**왜 `cargo install`을 wrap하나:** `cargo install turm-cli` (crates.io 미배포)과 `cargo install --path .` (workspace 루트는 virtual manifest)는 모두 실패합니다. 올바른 형태는 `cargo install --path turm-cli`이고, 스크립트가 이걸 wrap합니다.

**Info.plist 중복:** `run.sh`와 `install-macos.sh`가 동일 Info.plist를 인라인으로 포함합니다. 두 카피는 의도된 상태 (Rule of Three: 2회는 OK). 세 번째가 생기면 그때 템플릿화.

### Rust FFI (`turm-ffi` + `CTurmFFI`)

PR 1 / Tier 2.1 spike. macOS 앱이 shared Rust core(`turm-core`의 trigger engine, supervisor 등)를 향후 사용하려면 Swift ↔ Rust C-ABI 다리가 필요합니다. SwiftPM은 cargo를 prebuild로 직접 호출할 방법이 없어서 빌드 파이프라인이 두 단계로 갈라집니다:

```
cargo build --release -p turm-ffi   →  target/release/libturm_ffi.a
swift build                          →  Turm (links libturm_ffi.a)
```

`run.sh` / `install-macos.sh` 둘 다 두 단계를 묶어 줍니다. `swift build`만 단독으로 돌리면 link 단계에서 undefined-symbol 에러가 납니다 (clean target/ 상태에서).

**구성:**
- `turm-ffi/` — workspace member, `crate-type = ["staticlib"]`. `serde_json`만 의존. 4개의 `extern "C"` 심볼 노출:
  - `turm_ffi_version()` — 정적 C 문자열, 해제 불필요
  - `turm_ffi_call_json(input)` — JSON in / JSON out, 결과는 heap-alloc (caller가 free)
  - `turm_ffi_free_string(*mut c_char)` — `call_json` 결과 해제
  - `turm_ffi_last_error()` — thread-local 마지막 에러 메시지 (borrowed)
- `turm-macos/Sources/CTurmFFI/` — SwiftPM C target. `include/turm_ffi.h` (수동 declaration, `turm-ffi/src/lib.rs`와 동기화) + `include/module.modulemap` (`module CTurmFFI { ... }`) + `dummy.c` (SwiftPM이 header-only target은 link graph에 안 넣어서 placeholder C 파일 필요).
- `turm-macos/Package.swift` — `Turm` exec target에 `linkerSettings: [.unsafeFlags(["-L../target/release"]), .linkedLibrary("turm_ffi")]`. 상대경로 `../target/release`는 link 시점에 패키지 루트(`turm-macos/`) 기준으로 풀려 cargo workspace target 디렉토리를 가리킴.
- `turm-macos/Sources/Turm/FFIBridge.swift` — `TurmFFI` enum 파사드. C 포인터를 Swift 호출자에게 노출하지 않음. `String(cString:)` copy 후 즉시 `turm_ffi_free_string`으로 free, ownership round-trip 닫힘.

**Smoke test:** `AppDelegate.applicationDidFinishLaunching`의 `TurmFFI.runSmokeTest()`가 매 launch마다 stderr에 `[turm-ffi] version = ...` + `[turm-ffi] echo round-trip = ...`를 찍음. `echoed_at`는 Rust가 만든 unix-ms timestamp이므로 round-trip이 진짜로 일어나는지 확인 가능. Tier 2.4 (TriggerEngine via FFI)에서 실제 엔진 startup으로 교체 예정.

**Universal binary:** 현재 spike는 cargo 호스트 아키텍처(arm64) only. x86_64 사용자가 생기면 `cargo build --target aarch64-apple-darwin && cargo build --target x86_64-apple-darwin && lipo -create … -output target/release/libturm_ffi.a` recipe로 합치면 됨 (PR 1 시점에 deferred).

### Plugin Supervisor (`PluginManifest` + `PluginSupervisor`)

PR 3 / Tier 3 spike. Linux의 `service_supervisor.rs` (1794 LOC)를 native Swift로 최소 surface만 포팅. echo plugin 하나 굴리는 데 필요한 것만 들어감.

**Discovery:**
`PluginManifestStore.discover()`가 두 디렉토리를 union:
1. `~/Library/Application Support/turm/plugins/<name>/plugin.toml` (macOS-native, 우선)
2. `~/.config/turm/plugins/<name>/plugin.toml` (XDG fallback — Linux/macOS dotfile-sharing 사용자용)

같은 plugin name이 둘 다 있으면 macOS path가 이김. parse 실패한 manifest는 stderr에 로그 + skip.

**TOMLKit Decodable gotcha:**
TOMLKit의 Decoder는 `var foo: T = default` 디폴트 값을 무시함. Swift init feature지 Decodable feature가 아님. 누락된 키는 `keyNotFound` throw. serde의 `#[serde(default)]`처럼 동작하려면 `init(from:)`에서 `decodeIfPresent ?? <default>` 명시적으로. PR 3에서 echo manifest 디코딩 시 이 버그로 한 번 실패해서 잡았음.

**Spawn + init handshake:**
service.activation == "onStartup"인 service만 spawn (PR 5에서 `onAction:*` / `onEvent:*` 추가). `Process` + 3 `Pipe` (stdin/stdout/stderr), `executableURL`은 `<plugin_dir>/<service.exec>` 우선 → 없으면 `$PATH` 직접 lookup (`/usr/bin/which` shell-out 안 함 — `$PATH` 재평가가 process spawn 시점에 다시 일어나서). reader thread 먼저 띄우고 `{id, method: "initialize", params: {protocol_version: 1}}` 보낸 뒤 5s 시한으로 `InitBox.semaphore.wait()`. 응답 받으면 manifest의 provides[] subset인지 검증 (런타임이 manifest보다 더 많이 claim하면 reject — 다른 plugin 액션 shadow 위험), `ActionRegistry.register(name, ...)`로 등록, `{method: "initialized"}` notification 발사.

**Action dispatch:**
ActionRegistry handler는 `proc.invoke(action:params:completion:)` 호출. UUID 만들어 `pending[id] = completion` 저장 (NSLock-protected), `{id, method: "action.invoke", params: {name, params}}` stdin 전송, 즉시 리턴. reader thread가 응답에서 id 매칭 → completion(payload) 호출. payload는 `decodeResponse(obj)`가 `result` 또는 `RPCError`로 변환.

**Critical bug (verification 중 발견):**
초기 구현은 reader thread completion을 `DispatchQueue.main.async`로 main actor에 hop했음. 그런데 init handshake 중 main thread가 `initBox.semaphore.wait()` 으로 park돼 있어서 main에 hop된 작업이 영원히 안 깨어남 → deadlock. **수정:** completion은 reader thread에서 inline 호출. SocketServer leaf와 InitBox.resolve 둘 다 actor isolation 가정 없는 leaf (sema signal + class var set)이라 thread-safe. main actor 작업이 필요한 미래 completion은 자기 body에서 hop하면 됨.

**Shutdown:**
`applicationWillTerminate`에서 `pluginSupervisor.shutdown()`. 각 process에 `{method: "shutdown"}` 보내고 200ms 대기, 살아있으면 `process.terminate()`. pending request는 모두 `RPCError(plugin_shutdown)`로 reject. SIGTERM/SIGKILL은 applicationWillTerminate를 안 거치지만, 어차피 parent가 죽으면 plugin의 stdin이 EOF되면서 reader loop 종료 → `main()` return → 자연사.

**의도적으로 deferred (PR 4+):**
- restart-on-crash policy
- `event.publish` notification → EventBus 포워딩 (PR 5에서 trigger engine과 함께)
- `event.dispatch` (host → plugin) — echo는 subscribe 안 함
- provides[] cross-plugin 충돌 해결
- Process group (`setpgid(0,0)`) + PID file 기반 crash recovery (codex I5)
- 코드 서명 — 로컬 빌드는 quarantine xattr 없어서 Gatekeeper 패스, release tarball에서는 별도 처리 필요

**Install flow:**
`scripts/install-macos.sh` 가 `MACOS_PLUGINS=(echo)` 배열 돌면서 각각 `cargo build --release -p turm-plugin-<name>` → `~/Library/Application Support/turm/plugins/<name>/`에 manifest copy + binary copy (symlink 아님 — `git clean target/`로 silently 깨지지 않게). `--no-plugins` 플래그로 스킵 가능.

**End-to-end 검증:**
```bash
turmctl call system.list_actions
# {"count":3, "names":["echo.ping","system.ffi_test","system.list_actions"]}

turmctl call echo.ping --params '{"hello":"x","sleep_ms":200}'
# {"echoed":{"hello":"x","sleep_ms":200}, "from":"turm-plugin-echo"}  # 200ms+ blocking
```

**Activation 처리 (PR 4 확장):**
- `onStartup` → eager spawn (PR 3 원래 동작)
- `onAction:<glob>` → eager spawn + 로그 (`lazy not yet implemented, spawning eagerly`). 진짜 lazy는 placeholder handler 등록 → first call에서 spawn → 후속 호출 큐잉 패턴이 필요해서 PR 5 (트리거 엔진)와 같이 갈 예정. git처럼 spawn cost < 100ms인 plugin은 eager가 손해 거의 없음. Slack/Calendar처럼 startup이 무거운 (auth, network) plugin들이 들어오기 시작하면 진짜 lazy 구현이 의미를 가짐.
- `onEvent:<glob>` → 로그 후 skip. event-driven plugin을 eager로 띄우면 자원만 낭비 — 매칭되는 이벤트가 와야만 의미가 있고, 이벤트 라우팅은 트리거 엔진이 한다.

**MACOS_PLUGINS (현재 install-macos.sh에서 자동 빌드/설치):**
- `echo` (PR 3) — 프로토콜 sanity check
- `git` (PR 4) — 6개 액션 (`git.list_workspaces`/`list_worktrees`/`worktree_add`/`worktree_remove`/`current_branch`/`status`). cross-platform deps만 사용 (serde, serde_json, toml). `~/.config/turm/workspaces.toml` (또는 `TURM_GIT_WORKSPACES_FILE` env override)에서 워크스페이스 정의 읽음. config 없으면 list_workspaces가 빈 배열 반환 (`fatal_error: null`) — graceful.

**아직 enabled 못 된 plugins:**
- `kb`, `todo`, `bookmark` — `renameat2` / `O_NOFOLLOW` 같은 Linux-only filesystem primitives 의존. atomic-create / symlink-safety 백엔드를 Apple File System 호환 형태로 갈아엎어야 함.
- `slack`, `calendar`, `llm`, `discord` — `unix` cfg gate라 컴파일은 됨. 미검증: `keyring` `apple-native` Keychain prompt UX (특히 unsigned binary), 실제 OAuth 플로우.

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

**ActionRegistry seam (PR 2 / Tier 2.3):**
`handleCommand(method:params:completion:)`의 첫 줄이 `actionRegistry.tryDispatch(method, params:, completion:)`를 호출. 등록된 핸들러가 있으면 거기서 처리하고 리턴; 없으면 (`tryDispatch` returns `false`) 기존 hardcoded switch로 fall-through. PR 3(플러그인 슈퍼바이저)와 PR 5(트리거 엔진)가 이 registry에 자기 핸들러를 등록할 예정.

`registerBuiltinActions()`에서 launch 시점에 `system.*` 액션 등록:
- **`system.ffi_test`** — `params`(없으면 `{caller: "system.ffi_test"}`)를 `TurmFFI.callJSON`으로 통과시키고 `{echoed, ffi_version}` 반환. socket → registry → FFI → Rust → 다시 socket까지 entire seam을 단발 호출로 검증 가능. PR 5가 트리거 엔진 dispatch target 으로도 재사용 예정.
- **`system.list_actions`** — `{count, names: [...]}` 반환. 디버깅 + 추후 플러그인 등록 확인용.

```bash
turmctl call system.list_actions
# {"count": 2, "names": ["system.ffi_test", "system.list_actions"]}

turmctl call system.ffi_test --params '{"hello":"x"}'
# {"echoed": {"echoed_at": <ms>, "hello": "x"}, "ffi_version": "turm-ffi 0.1.0"}
```

`ActionRegistry`는 Linux full surface(`register_silent` / `register_blocking` / `invoke` / `try_invoke` / completion bus)의 일부만 가져옴. `register_blocking`은 의도적으로 보류 — Linux는 dispatch 즉시 리턴 후 worker thread에서 blocking 핸들러 실행하지만, macOS 소켓 dispatch가 main-actor completion 까지 socket thread를 세마포어로 잡아두는 모델이라 섞으면 데드락 위험 있음. PR 5에서 트리거 엔진 wiring할 때 async boundary 다시 설계.

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
두 가지 역할의 래퍼:
1. SwiftTerm의 `processTerminated` 미호출 버그 우회 — `installExitMonitor()`에서 별도 `DispatchSource.makeProcessSource`를 설치.
2. OSC 52 clipboard write를 정책으로 게이트 — `installDelegateProxy(policy:)`에서 `TurmTerminalDelegate` proxy를 SwiftTerm의 `terminalDelegate` slot에 install. SwiftTerm의 `clipboardCopy`가 `public`(non-`open`)이라 직접 override 불가하기 때문에 delegate slot 자체를 가로챔. 자세한 배경은 troubleshooting.md 참조.

**`TurmTerminalDelegate` (private proxy):**
`TerminalViewDelegate`를 구현. host(`LocalProcessTerminalView`)에 weak ref. `sizeChanged` / `setTerminalTitle` / `hostCurrentDirectoryUpdate` / `send` / `scrolled` / `rangeChanged`은 host의 `public` 구현에 forward (PTY winsize 갱신, 타이틀 업데이트, OSC 7, 키 입력 등 정상 동작). `clipboardCopy`만 `OSC52Policy`로 분기 — `.deny`(기본)는 stderr 한 줄 로그 + drop, `.allow`는 SwiftTerm 원래 동작 (`NSPasteboard.general`에 write). `requestOpenLink` / `bell` / `iTermContent`는 protocol extension의 default를 그대로 둠.

정책은 `[security] osc52 = "deny" | "allow"` (config), hot-reload는 `applyOSC52Policy(_:)` → `delegateProxy?.policy = ...`로 전달.

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

`~/.config/turm/config.toml`을 `LebJe/TOMLKit` (SwiftPM dep, 0.6.0)으로 디코드. `TurmConfig.parse`가 `TOMLDecoder().decode(RawConfig.self, …)`로 private shadow 타입에 매핑하고, 누락된 섹션·키는 모두 optional이라 `?? defaults.X` fallback로 처리.

**TOML 구조 (현재 macOS가 디코드하는 키만):**
```toml
[terminal]
shell = "/bin/zsh"
font_family = "JetBrains Mono"
font_size = 13

[theme]
name = "catppuccin-mocha"

[background]
path = "~/Pictures/wall.png"      # alias: image
tint = 0.6
opacity = 0.95

[security]
osc52 = "deny"                    # or "allow"
```

Linux 쪽 schema의 `[tabs]`, `[statusbar]`, `[keybindings]`, `[[triggers]]`는 macOS가 아직 안 읽지만 TOMLKit이 unknown key를 silently 무시하므로 Linux 사용자가 같은 config 파일을 그대로 쓸 수 있음. malformed TOML은 stderr에 `[turm] config.toml parse failed: …`를 찍고 defaults로 fallback (crash 안 함).

**snake_case 매핑:** TOMLKit 0.6에는 `keyDecodingStrategy`가 없어서 `font_family`, `font_size`만 `TerminalSection.CodingKeys`에서 명시적으로 매핑. 단일 단어 키는 그대로 둠.

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
| `webview.navigate` | `url`, `id?` | 웹뷰 URL 이동 (id 없으면 active fallback) |
| `webview.back` | `id?` | 뒤로 |
| `webview.forward` | `id?` | 앞으로 |
| `webview.reload` | `id?` | 새로고침 |
| `webview.execute_js` | `code`, `id?` (alias: `script`) | JS 평가 (비동기 응답) |
| `webview.get_content` | `id?` | 페이지 HTML 반환 (비동기) |
| `webview.devtools` | `id?`, `action?` (show/close/attach/detach/toggle) | Safari Web Inspector 토글 (close는 no-op) |
| `webview.state` | `id?` | url/title/can_go_back/forward/is_loading |

**`id` 파라미터**: 모든 `webview.*` 커맨드는 `params["id"]` (UUID)로 특정 패널을 지정할 수 있고, 생략하면 active webview로 fallback (Linux는 필수, macOS는 lenient default — Tier 1.6 plan 결정). `id`로 못 찾으면 `not_found`, 패널이 webview가 아니면 `wrong_panel_type` 에러 envelope 반환. `webview.execute_js`의 param 이름은 Linux/turm-cli convention인 `code`로 통일했고, 기존 macOS-only 호출자를 위해 `script`도 인식.

**에러 envelope**: 웹뷰 핸들러가 `RPCError(code:message:)`를 completion에 전달하면 `SocketServer.dispatch`가 감지해서 JSON-RPC 에러 형태(`{ok:false, error:{code, message}}`)로 wrap. Linux 코드와 일치 (`not_found`, `wrong_panel_type`, `invalid_params`, `no_active_webview`). 다른 핸들러는 기존 `(Any?) -> Void` completion을 그대로 사용.

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
- ~~**Double-click rename** — 탭 라벨 더블클릭으로 인라인 편집~~ ✅ 구현 완료
- **Pane focus navigation** — 키보드로 다음/이전 pane 포커스 이동
- **Background random rotation** — `background.next` 소켓 커맨드, `[background] directory` 설정
- ~~**Config hot-reload** — 파일 변경 감지 후 테마/설정 즉시 반영~~ ✅ 구현 완료

### Phase 5: Distribution & Ecosystem
- Session persistence / restore
- ~~Clipboard integration (OSC 52)~~ ✅ deny-by-default 구현 (`[security] osc52`); Linux 측 VTE는 이미 deny 기본
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
