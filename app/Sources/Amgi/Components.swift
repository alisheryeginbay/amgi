/// Last path component of a deck name: `science/physics` → `physics`.
func deckTitle(_ name: String) -> String {
    name.split(separator: "/").last.map(String.init) ?? name
}

/// Time spent studying, rounded to what matters at its size: "45 s", "23 min", "1 h 5 min".
/// The dashboard writes it the same way.
func formatStudyTime(seconds: UInt32) -> String {
    if seconds < 60 { return "\(seconds) s" }
    let minutes = (Int(seconds) + 30) / 60
    if minutes < 60 { return "\(minutes) min" }
    let (hours, rest) = minutes.quotientAndRemainder(dividingBy: 60)
    return rest == 0 ? "\(hours) h" : "\(hours) h \(rest) min"
}
