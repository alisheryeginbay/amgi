import SwiftUI

/// Last path component of a deck name: `science/physics` → `physics`.
func deckTitle(_ name: String) -> String {
    name.split(separator: "/").last.map(String.init) ?? name
}

/// A card's tags as capsules, in the surrounding font. The browse table draws its own,
/// in `CardTable`.
struct TagPills: View {
    let tags: [String]
    /// Lowercased tag → icon.
    let icons: [String: String]

    var body: some View {
        HStack(spacing: 4) {
            // By position: front matter can repeat a tag.
            ForEach(Array(tags.enumerated()), id: \.offset) { _, tag in
                Text([icons[tag.lowercased()], tag].compactMap(\.self).joined(separator: " "))
                    .lineLimit(1)
                    .padding(.horizontal, 7)
                    .padding(.vertical, 1)
                    .foregroundStyle(.secondary)
                    .background(.quaternary, in: .capsule)
            }
        }
    }
}
