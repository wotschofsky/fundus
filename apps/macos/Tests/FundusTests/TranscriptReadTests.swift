import XCTest
@testable import Fundus

final class TranscriptReadTests: XCTestCase {
    /// An answer without segments says they are unchanged since the key it was asked with. Once another
    /// answer replaced the rows, one like that would pair its recording with rows it never saw.
    func testAnAnswerWithoutSegmentsStandsOnlyBesideTheRowsItWasAskedAbout() {
        XCTAssertTrue(Detail.fits(hasSegments: false, key: "k1", held: "k1"))
        XCTAssertFalse(Detail.fits(hasSegments: false, key: "k1", held: "k2"))
        XCTAssertTrue(Detail.fits(hasSegments: true, key: "k1", held: "k2"))
        XCTAssertTrue(Detail.fits(hasSegments: true, key: "k1", held: nil))
    }
}
