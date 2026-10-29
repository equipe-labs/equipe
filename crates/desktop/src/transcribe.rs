//! A recording attached (a voice memo, a call) written down on this Mac, so a
//! member reads what was said: Apple's speech recognition, on the device only,
//! the way dictation in the box listens.

use std::path::Path;
use std::time::Duration;

/// Why a recording wasn't written down. The file stays attached either way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// Speech recognition is turned off for equipe in System Settings.
    Denied,
    /// No recognizer on this Mac for the language, or none on the device.
    Unsupported,
    /// Nothing said in it.
    NoSpeech,
    Failed(String),
}

/// How long to wait for a recording `length` seconds long: it is read faster
/// than it plays, so this is only for when the recognizer stops answering.
pub fn patience(length: Option<f64>) -> Duration {
    let length = length.filter(|l| l.is_finite() && *l > 0.).unwrap_or(600.);
    Duration::from_secs_f64((length * 1.5).clamp(60., 3. * 3600.) + 30.)
}

/// What was said in the recording at `path`, in `locale` ("ko-KR"). Blocks
/// until the recognizer is through: call it off the main thread.
pub fn transcribe(path: &Path, locale: &str, length: Option<f64>) -> Result<String, Failure> {
    #[cfg(target_os = "macos")]
    {
        sf::transcribe(path, locale, patience(length))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (path, locale, length);
        Err(Failure::Unsupported)
    }
}

/// The results, put together as they come. A long recording comes in
/// utterances: each result after a pause may start over from empty, so what
/// was said before it is kept.
#[derive(Debug, Default)]
struct Joiner {
    said: String,
    /// The last result that ended an utterance, until the next shows whether
    /// it carries it on or starts over.
    utterance: Option<String>,
    /// The latest result.
    now: String,
}

impl Joiner {
    fn result(&mut self, text: &str, ends_utterance: bool) {
        let before = std::mem::take(&mut self.now);
        match self.utterance.take() {
            Some(utterance) if starts_over(&utterance, text) => self.keep(&utterance),
            // Results after a pause start over, and only a final one says it
            // ended an utterance: one much shorter, unlike the last, is a new one.
            Some(_) => {}
            None if !before.is_empty() && starts_over(&before, text) && text.chars().count() * 10 < before.chars().count() * 7 => self.keep(&before),
            None => {}
        }
        if ends_utterance {
            self.utterance = Some(text.to_owned());
        }
        self.now = text.to_owned();
    }

    fn keep(&mut self, text: &str) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        if let (Some(before), Some(after)) = (self.said.chars().next_back(), text.chars().next())
            && needs_space(before, after)
        {
            self.said.push(' ');
        }
        self.said.push_str(text);
    }

    /// All of it: what was kept and the latest result.
    fn text(mut self) -> String {
        let now = std::mem::take(&mut self.now);
        self.keep(&now);
        self.said
    }
}

/// Whether `text`, the result after the one that ended `utterance`, starts a
/// new utterance instead of revising it: less than half of it in common.
fn starts_over(utterance: &str, text: &str) -> bool {
    let common = utterance.chars().zip(text.chars()).take_while(|(a, b)| a == b).count();
    common * 2 < utterance.chars().count()
}

fn needs_space(before: char, after: char) -> bool {
    let unspaced = |c: char| matches!(c, '\u{3000}'..='\u{30FF}' | '\u{3400}'..='\u{4DBF}' | '\u{4E00}'..='\u{9FFF}' | '\u{F900}'..='\u{FAFF}' | '\u{FF00}'..='\u{FFEF}');
    !before.is_whitespace() && !after.is_whitespace() && !matches!(after, ',' | '.' | '?' | '!' | ';' | ':' | ')') && !unspaced(before) && !unspaced(after)
}

#[cfg(target_os = "macos")]
mod sf {
    use super::*;
    use block2::RcBlock;
    use objc2::{AnyThread as _, runtime::NSObjectProtocol as _, sel};
    use objc2_foundation::{NSBundle, NSError, NSLocale, NSString, NSURL};
    use objc2_speech::{SFSpeechRecognitionResult, SFSpeechRecognizer, SFSpeechRecognizerAuthorizationStatus as Status, SFSpeechURLRecognitionRequest};
    use std::sync::mpsc;

    /// The error the recognizer reports when the audio held no speech.
    const NO_SPEECH: (&str, isize) = ("kAFAssistantErrorDomain", 1110);

    enum Heard {
        Result { text: String, last: bool, ends_utterance: bool },
        Error { domain: String, code: isize, message: String },
    }

    fn allowed() -> Result<(), Failure> {
        // Asking without the Info.plist line ends the process (a bare `cargo run`).
        if NSBundle::mainBundle().objectForInfoDictionaryKey(&NSString::from_str("NSSpeechRecognitionUsageDescription")).is_none() {
            return Err(Failure::Unsupported);
        }
        let status = match unsafe { SFSpeechRecognizer::authorizationStatus() } {
            Status::NotDetermined => {
                let (tx, rx) = mpsc::channel();
                let handler = RcBlock::new(move |status: Status| {
                    let _ = tx.send(status);
                });
                unsafe { SFSpeechRecognizer::requestAuthorization(&handler) };
                // The person is being asked: give them time to answer.
                rx.recv_timeout(Duration::from_secs(300)).unwrap_or(Status::NotDetermined)
            }
            status => status,
        };
        if status == Status::Authorized { Ok(()) } else { Err(Failure::Denied) }
    }

    pub fn transcribe(path: &Path, locale: &str, patience: Duration) -> Result<String, Failure> {
        allowed()?;
        let locale = NSLocale::initWithLocaleIdentifier(NSLocale::alloc(), &NSString::from_str(locale));
        let recognizer = unsafe { SFSpeechRecognizer::initWithLocale(SFSpeechRecognizer::alloc(), &locale) }.ok_or(Failure::Unsupported)?;
        // Never sent to Apple's servers, as dictation isn't.
        if !unsafe { recognizer.isAvailable() && recognizer.supportsOnDeviceRecognition() } {
            return Err(Failure::Unsupported);
        }
        let url = NSURL::fileURLWithPath(&NSString::from_str(path.to_str().ok_or_else(|| Failure::Failed("unreadable path".into()))?));
        let request = unsafe {
            let request = SFSpeechURLRecognitionRequest::initWithURL(SFSpeechURLRecognitionRequest::alloc(), &url);
            // Partial results show where an utterance ends, so a long one is kept whole.
            request.setShouldReportPartialResults(true);
            request.setRequiresOnDeviceRecognition(true);
            if request.respondsToSelector(sel!(setAddsPunctuation:)) {
                request.setAddsPunctuation(true);
            }
            request
        };
        let (tx, rx) = mpsc::channel::<Heard>();
        let handler = RcBlock::new(move |result: *mut SFSpeechRecognitionResult, error: *mut NSError| {
            // SAFETY: the framework passes nil or objects valid for the call.
            let (result, error) = unsafe { (result.as_ref(), error.as_ref()) };
            if let Some(result) = result {
                let heard = unsafe {
                    Heard::Result { text: result.bestTranscription().formattedString().to_string(), last: result.isFinal(), ends_utterance: result.speechRecognitionMetadata().is_some() }
                };
                let _ = tx.send(heard);
            }
            if let Some(error) = error {
                let _ = tx.send(Heard::Error { domain: error.domain().to_string(), code: error.code(), message: error.localizedDescription().to_string() });
            }
        });
        let task = unsafe { recognizer.recognitionTaskWithRequest_resultHandler(&request, &handler) };
        let until = std::time::Instant::now() + patience;
        let mut joiner = Joiner::default();
        let ended = loop {
            let left = until.saturating_duration_since(std::time::Instant::now());
            match rx.recv_timeout(left) {
                Ok(Heard::Result { text, last, ends_utterance }) => {
                    joiner.result(&text, ends_utterance);
                    if last {
                        break None;
                    }
                }
                Ok(Heard::Error { domain, code, message }) => break Some((domain == NO_SPEECH.0 && code == NO_SPEECH.1, message)),
                Err(_) => {
                    unsafe { task.cancel() };
                    break Some((false, "timed out".into()));
                }
            }
        };
        let text = joiner.text();
        match ended {
            // What was heard before it stopped is still worth having.
            _ if !text.is_empty() => Ok(text),
            None | Some((true, _)) => Err(Failure::NoSpeech),
            Some((false, message)) => Err(Failure::Failed(message)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_recording_keeps_every_utterance() {
        let mut j = Joiner::default();
        j.result("Hello", false);
        j.result("Hello world.", true);
        j.result("How", false);
        j.result("How are you?", true);
        j.result("Fine", false);
        assert_eq!(j.text(), "Hello world. How are you? Fine");
    }

    #[test]
    fn what_was_said_before_a_pause_is_kept_without_being_told() {
        // As a recording reads on the device: partial results only, each
        // pause starting over, no word on where an utterance ended.
        let mut j = Joiner::default();
        for partial in ["오늘", "오늘 회의는", "오늘 회의는 삼성동에서 했어요.", "다음", "다음 주에는", "다음 주에는 계약서를 보내요."] {
            j.result(partial, false);
        }
        j.result("마지막", false);
        assert_eq!(j.text(), "오늘 회의는 삼성동에서 했어요. 다음 주에는 계약서를 보내요. 마지막");
    }

    #[test]
    fn a_revised_utterance_is_kept_once() {
        let mut j = Joiner::default();
        j.result("Hello word", true);
        j.result("Hello world, how are", false);
        assert_eq!(j.text(), "Hello world, how are");
    }

    #[test]
    fn korean_and_chinese_join_as_written() {
        let mut j = Joiner::default();
        j.result("안녕하세요.", true);
        j.result("오늘 회의", false);
        assert_eq!(j.text(), "안녕하세요. 오늘 회의");
        let mut j = Joiner::default();
        j.result("今天天气很好。", true);
        j.result("明天", false);
        assert_eq!(j.text(), "今天天气很好。明天");
    }

    #[test]
    fn nothing_heard_is_empty() {
        assert_eq!(Joiner::default().text(), "");
    }

    #[test]
    fn patience_grows_with_the_recording() {
        assert_eq!(patience(Some(10.)), Duration::from_secs(90));
        assert_eq!(patience(Some(600.)), Duration::from_secs(930));
        assert_eq!(patience(None), patience(Some(600.)));
        assert!(patience(Some(1e9)) <= Duration::from_secs(3 * 3600 + 30));
    }
}
