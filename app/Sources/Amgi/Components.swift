import Foundation

/// Last path component of a deck name: `science/physics` → `physics`.
func deckTitle(_ name: String) -> String {
    name.split(separator: "/").last.map(String.init) ?? name
}
