import AppKit

// MARK: - TabButton

private final class TabButton: NSView {
    var onSelect: (() -> Void)?
    var onClose: (() -> Void)?

    private let titleLabel = NSTextField(labelWithString: "")
    private let closeBtn = NSButton()
    private var trackingArea: NSTrackingArea?
    private let theme: TurmTheme

    private(set) var isActive = false
    private var isHovered = false

    var title: String {
        get { titleLabel.stringValue }
        set { titleLabel.stringValue = newValue }
    }

    init(theme: TurmTheme) {
        self.theme = theme
        super.init(frame: .zero)
        setupView()
    }

    @available(*, unavailable)
    required init?(coder _: NSCoder) {
        fatalError()
    }

    private func setupView() {
        wantsLayer = true
        layer?.cornerRadius = 4

        // Title label
        titleLabel.font = NSFont.systemFont(ofSize: 12)
        titleLabel.lineBreakMode = .byTruncatingTail
        titleLabel.translatesAutoresizingMaskIntoConstraints = false
        addSubview(titleLabel)

        // Close button
        closeBtn.title = "×"
        closeBtn.isBordered = false
        closeBtn.font = NSFont.systemFont(ofSize: 14, weight: .light)
        closeBtn.target = self
        closeBtn.action = #selector(closeTapped)
        closeBtn.translatesAutoresizingMaskIntoConstraints = false
        closeBtn.setContentHuggingPriority(.required, for: .horizontal)
        addSubview(closeBtn)

        NSLayoutConstraint.activate([
            closeBtn.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -6),
            closeBtn.centerYAnchor.constraint(equalTo: centerYAnchor),
            closeBtn.widthAnchor.constraint(equalToConstant: 16),
            closeBtn.heightAnchor.constraint(equalToConstant: 16),

            titleLabel.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            titleLabel.centerYAnchor.constraint(equalTo: centerYAnchor),
            titleLabel.trailingAnchor.constraint(equalTo: closeBtn.leadingAnchor, constant: -4),
        ])

        let click = NSClickGestureRecognizer(target: self, action: #selector(selectTapped))
        addGestureRecognizer(click)

        applyStyle()
    }

    func setActive(_ active: Bool) {
        isActive = active
        applyStyle()
    }

    private func applyStyle() {
        let bgColor: NSColor
        let textColor: NSColor
        let closeTint: NSColor

        if isActive {
            bgColor = theme.surface2.nsColor
            textColor = theme.text.nsColor
            closeTint = theme.subtext0.nsColor
        } else if isHovered {
            bgColor = theme.surface1.nsColor
            textColor = theme.subtext1.nsColor
            closeTint = theme.subtext0.nsColor
        } else {
            bgColor = .clear
            textColor = theme.subtext0.nsColor
            closeTint = .clear
        }

        layer?.backgroundColor = bgColor.cgColor
        titleLabel.textColor = textColor
        closeBtn.contentTintColor = closeTint
    }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let old = trackingArea { removeTrackingArea(old) }
        let area = NSTrackingArea(
            rect: bounds,
            options: [.mouseEnteredAndExited, .activeInActiveApp],
            owner: self,
            userInfo: nil,
        )
        addTrackingArea(area)
        trackingArea = area
    }

    override func mouseEntered(with _: NSEvent) {
        isHovered = true
        applyStyle()
    }

    override func mouseExited(with _: NSEvent) {
        isHovered = false
        applyStyle()
    }

    @objc private func selectTapped() {
        onSelect?()
    }

    @objc private func closeTapped() {
        onClose?()
    }
}

// MARK: - TabBarView

final class TabBarView: NSView {
    static let height: CGFloat = 36

    var onSelectTab: ((Int) -> Void)?
    var onCloseTab: ((Int) -> Void)?
    var onNewTab: (() -> Void)?

    private let stackView = NSStackView()
    private let scrollView = NSScrollView()
    private let addButton = NSButton()
    private var tabButtons: [TabButton] = []
    private let theme: TurmTheme

    init(theme: TurmTheme) {
        self.theme = theme
        super.init(frame: .zero)
        setupView()
    }

    @available(*, unavailable)
    required init?(coder _: NSCoder) {
        fatalError()
    }

    private func setupView() {
        wantsLayer = true
        layer?.backgroundColor = theme.surface0.nsColor.cgColor

        // Bottom border
        let border = NSView()
        border.wantsLayer = true
        border.layer?.backgroundColor = theme.overlay0.nsColor.cgColor
        border.translatesAutoresizingMaskIntoConstraints = false
        addSubview(border)
        NSLayoutConstraint.activate([
            border.leadingAnchor.constraint(equalTo: leadingAnchor),
            border.trailingAnchor.constraint(equalTo: trailingAnchor),
            border.bottomAnchor.constraint(equalTo: bottomAnchor),
            border.heightAnchor.constraint(equalToConstant: 1),
        ])

        // Tab stack
        stackView.orientation = .horizontal
        stackView.spacing = 2
        stackView.edgeInsets = NSEdgeInsets(top: 4, left: 4, bottom: 4, right: 4)
        stackView.translatesAutoresizingMaskIntoConstraints = false

        // Scroll view wraps the stack (horizontal scroll if many tabs)
        scrollView.documentView = stackView
        scrollView.hasHorizontalScroller = false
        scrollView.hasVerticalScroller = false
        scrollView.drawsBackground = false
        scrollView.translatesAutoresizingMaskIntoConstraints = false
        addSubview(scrollView)

        // "+" add button
        addButton.title = "+"
        addButton.isBordered = false
        addButton.font = NSFont.systemFont(ofSize: 16, weight: .light)
        addButton.contentTintColor = theme.subtext0.nsColor
        addButton.target = self
        addButton.action = #selector(addTabTapped)
        addButton.translatesAutoresizingMaskIntoConstraints = false
        addSubview(addButton)

        NSLayoutConstraint.activate([
            scrollView.leadingAnchor.constraint(equalTo: leadingAnchor),
            scrollView.topAnchor.constraint(equalTo: topAnchor),
            scrollView.bottomAnchor.constraint(equalTo: border.topAnchor),
            scrollView.trailingAnchor.constraint(equalTo: addButton.leadingAnchor, constant: -4),

            addButton.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -8),
            addButton.centerYAnchor.constraint(equalTo: centerYAnchor),
            addButton.widthAnchor.constraint(equalToConstant: 28),
        ])

        // Stack width tracks scroll view
        stackView.widthAnchor.constraint(greaterThanOrEqualTo: scrollView.widthAnchor).isActive = true
    }

    // MARK: - Public API

    func setTabs(titles: [String], activeIndex: Int) {
        // Remove old buttons
        tabButtons.forEach { $0.removeFromSuperview() }
        tabButtons.removeAll()
        stackView.arrangedSubviews.forEach { stackView.removeArrangedSubview($0); $0.removeFromSuperview() }

        for (i, title) in titles.enumerated() {
            let btn = makeTabButton(index: i, title: title, active: i == activeIndex)
            stackView.addArrangedSubview(btn)
            tabButtons.append(btn)

            NSLayoutConstraint.activate([
                btn.widthAnchor.constraint(greaterThanOrEqualToConstant: 80),
                btn.widthAnchor.constraint(lessThanOrEqualToConstant: 200),
                btn.heightAnchor.constraint(equalToConstant: TabBarView.height - 8),
            ])
        }
    }

    func updateTitle(_ title: String, at index: Int) {
        guard index < tabButtons.count else { return }
        tabButtons[index].title = title
    }

    // MARK: - Private

    private func makeTabButton(index: Int, title: String, active: Bool) -> TabButton {
        let btn = TabButton(theme: theme)
        btn.title = title
        btn.setActive(active)
        btn.onSelect = { [weak self] in self?.onSelectTab?(index) }
        btn.onClose = { [weak self] in self?.onCloseTab?(index) }
        return btn
    }

    @objc private func addTabTapped() {
        onNewTab?()
    }
}
