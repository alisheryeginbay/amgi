import SwiftUI

/// Last path component of a deck name: `science/physics` → `physics`.
func deckTitle(_ name: String) -> String {
    name.split(separator: "/").last.map(String.init) ?? name
}

/// A card's tags as capsules, in the surrounding font.
struct TagPills: View {
    let tags: [String]

    var body: some View {
        HStack(spacing: 4) {
            ForEach(tags, id: \.self) { tag in
                Text(tag)
                    .lineLimit(1)
                    .padding(.horizontal, 7)
                    .padding(.vertical, 1)
                    .foregroundStyle(.secondary)
                    .background(.quaternary, in: .capsule)
            }
        }
    }
}
