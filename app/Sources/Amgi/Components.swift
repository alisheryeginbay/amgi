import AmgiCore
import SwiftUI

/// Last path component of a deck name: `science/physics` → `physics`.
func deckTitle(_ name: String) -> String {
    name.split(separator: "/").last.map(String.init) ?? name
}

/// Due and new counts shown at the trailing edge of a row.
struct DeckCounts: View {
    let due: UInt32
    let new: UInt32

    var body: some View {
        HStack(spacing: 8) {
            if new > 0 { Text(new, format: .number).foregroundStyle(.blue) }
            if due > 0 { Text(due, format: .number).foregroundStyle(.secondary) }
        }
        .font(.callout)
        .monospacedDigit()
        .help("\(due) due, \(new) new")
    }
}
