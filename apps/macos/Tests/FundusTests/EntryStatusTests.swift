import XCTest
@testable import Fundus

final class EntryStatusTests: XCTestCase {
    /// The vault (or a local record updated on Resume) can carry the step a recording last ran
    /// in. Only a running recording is in that step.
    func testWaitingAndStoppedRecordingsDoNotShowTheirLastStep() {
        XCTAssertEqual(Entry.describe(stage: "failed", activeStage: "transcribing", pauseRequested: false), "Processing stopped")
        XCTAssertEqual(Entry.describe(stage: "paused", activeStage: "enriching", pauseRequested: nil), "Paused")
        XCTAssertEqual(Entry.describe(stage: "queued", activeStage: "transcribing", pauseRequested: false), "Waiting to process")
        XCTAssertEqual(Entry.describe(stage: "complete", activeStage: "indexing", pauseRequested: false), "Ready")
    }

    func testRunningRecordingsShowTheirStep() {
        XCTAssertEqual(Entry.describe(stage: "transcribing", activeStage: "transcribing", pauseRequested: false), "Transcribing")
        XCTAssertEqual(Entry.describe(stage: "uploading", activeStage: nil, pauseRequested: nil), "Uploading")
        XCTAssertEqual(Entry.describe(stage: "transcribing", activeStage: "transcribing", pauseRequested: true), "Pausing after this step…")
    }
}
