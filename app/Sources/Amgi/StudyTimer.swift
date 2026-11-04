import AppKit

/// Times the card on screen, for the review log.
///
/// The timer reads `SuspendingClock`, a monotonic clock: it counts up steadily
/// whatever happens to the date and time, so setting the clock, travelling across
/// time zones or a network time sync can't make a review negative or hours long, and
/// it stops while the Mac sleeps. On top of that the timer pauses while nobody can
/// be studying: Amgi isn't the active app, its window
/// is minimized, closed or covered, or the screen is asleep or locked. The core caps
/// what's left at a minute, for someone who walks away with the card up.
@MainActor
final class StudyTimer {
    private let clock = SuspendingClock()
    /// The card being timed.
    private var cardID: String?
    /// Time counted before `since`.
    private var counted: Duration = .zero
    /// When the timer last started running; `nil` while paused.
    private var since: SuspendingClock.Instant?
    /// Times of the answers given, latest last, so that one taken back with ⌘Z picks
    /// up where it left off.
    private var answered: [(cardID: String, time: Duration)] = []

    /// The view the card shows in, whose window must be on screen.
    weak var view: NSView? {
        didSet { update() }
    }
    private var appActive = NSApplication.shared.isActive
    private var screenAsleep = false
    private var screenLocked = false
    private var sessionInactive = false

    init() {
        let center = NotificationCenter.default
        observe(center, NSApplication.didBecomeActiveNotification) { $0.appActive = true }
        observe(center, NSApplication.didResignActiveNotification) { $0.appActive = false }
        // Minimized, closed, or covered by another window.
        observe(center, NSWindow.didChangeOcclusionStateNotification) { _ in }
        let workspace = NSWorkspace.shared.notificationCenter
        observe(workspace, NSWorkspace.screensDidSleepNotification) { $0.screenAsleep = true }
        observe(workspace, NSWorkspace.screensDidWakeNotification) { $0.screenAsleep = false }
        // Another user switched in with fast user switching.
        observe(workspace, NSWorkspace.sessionDidResignActiveNotification) { $0.sessionInactive = true }
        observe(workspace, NSWorkspace.sessionDidBecomeActiveNotification) { $0.sessionInactive = false }
        let distributed = DistributedNotificationCenter.default()
        observe(distributed, Notification.Name("com.apple.screenIsLocked")) { $0.screenLocked = true }
        observe(distributed, Notification.Name("com.apple.screenIsUnlocked")) { $0.screenLocked = false }
    }

    /// Time `cardID` from now, unless it is the card already timed; `nil` times nothing.
    /// Showing a card's answer keeps its timer going.
    func show(_ cardID: String?) {
        guard cardID != self.cardID else { return }
        self.cardID = cardID
        counted = .zero
        since = nil
        update()
    }

    /// Time spent on `cardID` in milliseconds, for answering it; `nil` if another card
    /// is timed. The timer then stops until the next card shows.
    func answer(_ cardID: String) -> UInt32? {
        guard cardID == self.cardID else { return nil }
        let time = elapsed
        answered.append((cardID, time))
        if answered.count > 100 { answered.removeFirst() }
        show(nil)
        return UInt32(clamping: Int64((time / .milliseconds(1)).rounded()))
    }

    /// Show `cardID` again after its answer was taken back, carrying on from the time
    /// that answer had; `nil` when the card can't be shown, having left its file.
    func undo(_ cardID: String?) {
        let last = answered.popLast()
        show(nil)
        show(cardID)
        if let last, last.cardID == cardID { counted = last.time }
    }

    /// Forget everything timed, as when the folder closes.
    func reset() {
        answered = []
        show(nil)
    }

    private var elapsed: Duration {
        counted + (since.map { $0.duration(to: clock.now) } ?? .zero)
    }

    private var windowVisible: Bool {
        guard let window = view?.window else { return false }
        return window.isVisible && window.occlusionState.contains(.visible)
    }

    /// Bank the time run so far, then run again only if someone can be studying.
    private func update() {
        let now = clock.now
        if let since { counted += since.duration(to: now) }
        let running = cardID != nil && appActive && windowVisible
            && !screenAsleep && !screenLocked && !sessionInactive
        since = running ? now : nil
    }

    private func observe(_ center: NotificationCenter, _ name: Notification.Name, _ change: @escaping @MainActor (StudyTimer) -> Void) {
        // Observers live as long as the app's one timer, so they are never removed.
        _ = center.addObserver(forName: name, object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated {
                guard let self else { return }
                change(self)
                self.update()
            }
        }
    }
}
