use jarvis_lib::dictation::reconcile::{TranscriptReconciler, ReconcileResult};
use jarvis_lib::dictation::injector::{apply_transcript_with_kind, MockTextInjector};
use jarvis_lib::audio::transcript_event::TranscriptPartialKind;

fn main() {
    let mut r = TranscriptReconciler::new();
    let mut m = MockTextInjector::new();
    let steps = [
        ("Hello. what are we saying? right now?", TranscriptPartialKind::Growth),
        ("Hello. what are we saying? right now? Let's say some other stuff.", TranscriptPartialKind::Growth),
        ("So what are we saying right now? Let's say some other stuff.", TranscriptPartialKind::Revision),
    ];
    for (text, kind) in steps {
        let result = r.apply_with_kind(text, kind);
        println!("kind={kind:?} text={text:?} => {result:?}");
        apply_transcript_with_kind(&mut r, text, kind, &mut m).unwrap();
        println!("  screen={:?} segment={:?}", r.screen_text(), r.segment_injected());
        println!("  type_calls={:?} backspaces={:?}", m.type_calls, m.backspace_calls);
    }
}
