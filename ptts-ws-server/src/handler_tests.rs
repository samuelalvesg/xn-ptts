use super::*;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::task::{Context, Poll};

struct StalledSocket(Arc<tokio::sync::Notify>);

impl futures_util::Sink<Message> for StalledSocket {
    type Error = std::io::Error;

    fn poll_ready(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.0.notify_one();
        Poll::Pending
    }

    fn start_send(self: Pin<&mut Self>, _: Message) -> Result<(), Self::Error> {
        unreachable!("the socket never becomes ready")
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Pending
    }

    fn poll_close(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }
}

struct EndlessAudio {
    produced: Arc<AtomicUsize>,
    dropped: Arc<AtomicBool>,
    full: Arc<tokio::sync::Notify>,
}

impl Iterator for EndlessAudio {
    type Item = ptts::Result<Vec<f32>>;

    fn next(&mut self) -> Option<Self::Item> {
        let n = self.produced.fetch_add(1, Ordering::SeqCst) + 1;
        if n == REPLY_QUEUE + AUDIO_QUEUE + 3 {
            self.full.notify_one();
        }
        Some(Ok(vec![0.1; 480]))
    }
}

impl Drop for EndlessAudio {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn a_slow_socket_bounds_audio_and_disconnect_joins_the_producer() {
    let (reply_tx, reply_rx) = mpsc::channel(REPLY_QUEUE);
    let (audio_tx, mut audio_rx) = mpsc::channel(AUDIO_QUEUE);
    let (cancel_tx, mut cancel_rx) = watch::channel(false);
    let stalled = Arc::new(tokio::sync::Notify::new());
    let forwarder = tokio::spawn(forward_replies(
        StalledSocket(stalled.clone()),
        reply_rx,
        cancel_rx.clone(),
        SEND_TIMEOUT,
    ));
    let produced = Arc::new(AtomicUsize::new(0));
    let dropped = Arc::new(AtomicBool::new(false));
    let full = Arc::new(tokio::sync::Notify::new());
    let stream =
        EndlessAudio { produced: produced.clone(), dropped: dropped.clone(), full: full.clone() };
    let generation = tokio::spawn(async move {
        let producer = tokio::task::spawn_blocking(move || drain_audio(stream, audio_tx));
        let mut encoder = Encoder::new(Format::default(), 480, 24000).unwrap();
        let outcome =
            forward_audio(&mut audio_rx, 480, 0, &mut encoder, &reply_tx, &mut cancel_rx).await;
        drop(audio_rx);
        producer.await.unwrap().unwrap();
        outcome.unwrap();
    });
    tokio::time::timeout(Duration::from_secs(2), stalled.notified()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), full.notified()).await.unwrap();
    // One chunk is in each forwarding stage and one is blocked in the producer.
    assert_eq!(produced.load(Ordering::SeqCst), REPLY_QUEUE + AUDIO_QUEUE + 3);
    cancel_tx.send(true).unwrap();
    tokio::time::timeout(Duration::from_secs(2), generation).await.unwrap().unwrap();
    forwarder.await.unwrap().unwrap();
    assert!(dropped.load(Ordering::SeqCst));
}

#[tokio::test]
async fn a_stalled_socket_times_out_and_closes_the_reply_channel() {
    let (tx, rx) = mpsc::channel(REPLY_QUEUE);
    let (_cancel_tx, cancel_rx) = watch::channel(false);
    tx.send(TtsReply::EndOfStream).await.unwrap();
    let error = forward_replies(
        StalledSocket(Arc::new(tokio::sync::Notify::new())),
        rx,
        cancel_rx,
        Duration::from_millis(10),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("stopped reading"));
    assert!(tx.is_closed());
}

#[tokio::test]
async fn normal_end_of_stream_drains_queued_replies_in_order() {
    let output = Arc::new(std::sync::Mutex::new(Vec::new()));
    let socket = futures_util::sink::unfold(output.clone(), |output, msg| async move {
        let Message::Text(text) = msg else { panic!("expected text reply") };
        output.lock().unwrap().push(text.to_string());
        Ok::<_, std::convert::Infallible>(output)
    });
    let (tx, rx) = mpsc::channel(REPLY_QUEUE);
    let (_cancel_tx, cancel_rx) = watch::channel(false);
    tx.send(TtsReply::Flushed { flush_id: 7 }).await.unwrap();
    tx.send(TtsReply::EndOfStream).await.unwrap();
    drop(tx);
    forward_replies(Box::pin(socket), rx, cancel_rx, SEND_TIMEOUT).await.unwrap();
    let output = output.lock().unwrap();
    assert_eq!(output.len(), 2);
    assert!(matches!(serde_json::from_str(&output[0]).unwrap(), TtsReply::Flushed { flush_id: 7 }));
    assert!(matches!(serde_json::from_str(&output[1]).unwrap(), TtsReply::EndOfStream));
}

#[test]
fn pending_text_is_limited_in_characters_and_a_rejected_append_preserves_it() {
    let mut buffer = "é".repeat(MAX_INPUT_CHARS - 1);
    assert!(append_text(&mut buffer, "🙂"));
    let before = buffer.clone();
    assert!(!append_text(&mut buffer, "x"));
    assert_eq!(buffer, before);
    buffer.clear();
    assert!(append_text(&mut buffer, "Hello."));
}

#[tokio::test(start_paused = true)]
async fn a_full_request_queue_waits_without_cancelling_a_healthy_client() {
    use futures_util::StreamExt;

    let queued = Arc::new(tokio::sync::Notify::new());
    let observed = Arc::new(AtomicUsize::new(0));
    let observed_reader = observed.clone();
    let queued_reader = queued.clone();
    let socket = futures_util::stream::iter(
        (0..REQUEST_QUEUE + 2)
            .map(|i| Ok::<_, std::io::Error>(Message::Text(i.to_string().into()))),
    )
    .inspect(move |_| {
        if observed_reader.fetch_add(1, Ordering::SeqCst) == REQUEST_QUEUE {
            queued_reader.notify_one();
        }
    });
    let (tx, mut rx) = mpsc::channel(REQUEST_QUEUE);
    let (cancel_tx, cancel_rx) = watch::channel(false);
    let reader = tokio::spawn(read_requests(socket, tx, cancel_tx));
    tokio::time::timeout(Duration::from_secs(1), queued.notified()).await.unwrap();
    assert_eq!(rx.len(), REQUEST_QUEUE);
    assert_eq!(observed.load(Ordering::SeqCst), REQUEST_QUEUE + 1);

    // Healthy clients can queue text while an earlier flush takes longer than a write timeout.
    tokio::time::advance(SEND_TIMEOUT + Duration::from_secs(1)).await;
    tokio::task::yield_now().await;
    assert!(!reader.is_finished());
    assert!(!*cancel_rx.borrow());
    assert_eq!(observed.load(Ordering::SeqCst), REQUEST_QUEUE + 1);

    for i in 0..REQUEST_QUEUE + 2 {
        let Message::Text(text) = rx.recv().await.unwrap() else { panic!("expected text") };
        assert_eq!(text.as_str(), i.to_string());
    }
    reader.await.unwrap();
    assert!(*cancel_rx.borrow(), "EOF still cancels the session");
}

#[tokio::test]
async fn closing_a_full_request_queue_releases_the_reader() {
    use futures_util::StreamExt;

    let full = Arc::new(tokio::sync::Notify::new());
    let signal = full.clone();
    let observed = AtomicUsize::new(0);
    let socket = futures_util::stream::iter(
        (0..REQUEST_QUEUE + 1).map(|_| Ok::<_, std::io::Error>(Message::Text("text".into()))),
    )
    .inspect(move |_| {
        if observed.fetch_add(1, Ordering::SeqCst) == REQUEST_QUEUE {
            signal.notify_one();
        }
    });
    let (tx, rx) = mpsc::channel(REQUEST_QUEUE);
    let (cancel_tx, cancel_rx) = watch::channel(false);
    let reader = tokio::spawn(read_requests(socket, tx, cancel_tx));
    tokio::time::timeout(Duration::from_secs(2), full.notified()).await.unwrap();
    drop(rx);
    tokio::time::timeout(Duration::from_secs(2), reader).await.unwrap().unwrap();
    assert!(*cancel_rx.borrow());
}

mod language_per_session {
    use super::*;
    use crate::model::load_ptts;
    use ptts::preprocess::Lang;
    use std::collections::HashMap;

    #[test]
    fn the_session_language_wins_over_the_server_default_with_multilang() {
        let en = Some(Normalize::for_lang(Lang::En));
        let norm = |lang, default| session_normalize(lang, default, Rules::DEFAULT, true);
        assert_eq!(norm(Some("pt"), en).unwrap().as_str(), "pt");
        assert_eq!(norm(None, en).unwrap().as_str(), "en");
        assert_eq!(norm(Some(""), en).unwrap().as_str(), "en");
    }

    #[test]
    fn without_multilang_only_the_servers_language_is_accepted() {
        let en = Some(Normalize::for_lang(Lang::En));
        let norm = |lang| session_normalize(lang, en, Rules::DEFAULT, false);
        assert_eq!(norm(Some("en")).unwrap().as_str(), "en");
        assert_eq!(norm(None).unwrap().as_str(), "en");
        let message = norm(Some("pt")).unwrap_err();
        assert!(message.contains("'en'") && message.contains("--multilang"), "{message}");
    }

    #[test]
    fn a_session_cannot_switch_normalization_off() {
        let en = Some(Normalize::for_lang(Lang::En));
        for multilang in [false, true] {
            for lang in ["none", "off", " Off "] {
                let message =
                    session_normalize(Some(lang), en, Rules::DEFAULT, multilang).unwrap_err();
                assert!(message.contains("not accepted"), "{lang}: {message}");
            }
        }
        // A server started with --lang none still serves unnormalized text to sessions with no lang.
        let off = Some(Normalize::OFF);
        assert_eq!(session_normalize(None, off, Rules::DEFAULT, false).unwrap(), Normalize::OFF);
    }

    #[test]
    fn no_language_anywhere_is_an_error_that_says_what_to_send() {
        let message = session_normalize(None, None, Rules::DEFAULT, true).unwrap_err();
        assert!(message.contains("lang") && message.contains("--lang"), "{message}");
        assert!(session_normalize(Some("xx"), None, Rules::DEFAULT, true).is_err());
    }

    #[test]
    fn a_voice_must_belong_to_the_language_or_declare_none() {
        let langs = HashMap::from([("Zoey".to_string(), "en".to_string())]);
        let (en, pt) = (Normalize::for_lang(Lang::En), Normalize::for_lang(Lang::Pt));
        assert!(voice_fits_lang(&langs, "Zoey", en));
        assert!(!voice_fits_lang(&langs, "Zoey", pt));
        assert!(voice_fits_lang(&langs, "Zoey", Normalize::OFF));
        assert!(voice_fits_lang(&langs, "default", pt), "no declared language fits all");
    }

    async fn setup_and_speak(app: &AppState, lang: Option<&str>, text: &str) -> Vec<u8> {
        speak_as(app, lang, None, text).await.unwrap()
    }

    async fn speak_as(
        app: &AppState,
        lang: Option<&str>,
        voice: Option<&str>,
        text: &str,
    ) -> Result<Vec<u8>, (u32, String)> {
        let (req_tx, mut req_rx) = mpsc::channel(REQUEST_QUEUE);
        let (reply_tx, mut reply_rx) = mpsc::channel(256);
        let (_cancel_tx, mut cancel_rx) = watch::channel(false);
        let lang = lang.map_or("null".to_string(), |l| format!("\"{l}\""));
        let msgs = [
            serde_json::json!({"type": "setup", "output_format": "pcm", "voice": voice})
                .to_string()
                .replacen('}', &format!(r#","lang":{lang}}}"#), 1),
            serde_json::json!({"type": "text", "text": text}).to_string(),
            r#"{"type":"end_of_stream"}"#.to_string(),
        ];
        for m in msgs {
            req_tx.send(Message::Text(m.into())).await.unwrap();
        }
        drop(req_tx);
        let app = app.clone();
        let run =
            tokio::spawn(
                async move { run_session(app, &mut req_rx, &reply_tx, &mut cancel_rx).await },
            );
        let mut pcm = Vec::new();
        while let Some(reply) = reply_rx.recv().await {
            match reply {
                TtsReply::Audio { audio, .. } => {
                    use base64::Engine;
                    pcm.extend(base64::engine::general_purpose::STANDARD.decode(audio).unwrap());
                }
                TtsReply::Error { message, code } => return Err((code, message)),
                _ => {}
            }
        }
        run.await.unwrap().unwrap();
        Ok(pcm)
    }

    /// Mono s16le at `rate` as a WAV file.
    fn wav(pcm: &[u8], rate: u32) -> Vec<u8> {
        let mut out = Vec::with_capacity(44 + pcm.len());
        out.extend(b"RIFF");
        out.extend((36 + pcm.len() as u32).to_le_bytes());
        out.extend(b"WAVEfmt ");
        out.extend(16u32.to_le_bytes());
        out.extend([1, 0, 1, 0]);
        out.extend(rate.to_le_bytes());
        out.extend((rate * 2).to_le_bytes());
        out.extend([2, 0, 16, 0]);
        out.extend(b"data");
        out.extend((pcm.len() as u32).to_le_bytes());
        out.extend(pcm);
        out
    }

    /// One instance, one set of weights, two simultaneous connections in different languages.
    #[tokio::test]
    #[ignore = "requires PTTS_TEST_MODEL: a multilingual checkpoint folder or hf:// repo"]
    async fn one_instance_serves_two_languages() {
        let model = std::env::var("PTTS_TEST_MODEL").expect("set PTTS_TEST_MODEL");
        let app = load_ptts(
            &model,
            None,
            None,
            ptts::synth::DeviceKind::Cpu,
            ptts::synth::Quant::Q80,
            0.3,
            7,
            1024,
            None,
            Rules::DEFAULT,
            true,
            &[],
            std::num::NonZeroUsize::new(2).unwrap(),
        )
        .await
        .unwrap();
        let cases = [
            ("pt1", "pt", "Temos R$ 10,50 e a Dra. Ana, em 2 de março, às 15h."),
            ("en1", "en", "We have $10.50 and Dr. Smith, on March 2nd at 3pm."),
            ("pt2", "pt", "We have $10.50 and Dr. Smith, on March 2nd at 3pm."),
            ("en2", "en", "Temos R$ 10,50 e a Dra. Ana, em 2 de março, às 15h."),
        ];
        // Two connections at once, as the instance serves them: each pair runs concurrently.
        for pair in cases.chunks(2) {
            let app = &app;
            let runs = pair.iter().map(|&(name, lang, text)| async move {
                (name, lang, setup_and_speak(app, Some(lang), text).await)
            });
            for (name, lang, pcm) in futures_util::future::join_all(runs).await {
                assert!(!pcm.is_empty(), "{name}");
                // Not silence.
                assert!(pcm.as_chunks::<2>().0.iter().any(|b| i16::from_le_bytes(*b).abs() > 300));
                if let Ok(dir) = std::env::var("PTTS_TEST_WAV_DIR") {
                    std::fs::create_dir_all(&dir).unwrap();
                    let file = format!("{dir}/{name}-{lang}.wav");
                    std::fs::write(file, wav(&pcm, app.sample_rate)).unwrap();
                }
            }
        }
    }

    /// Catalog voices speak with their own accent; `lang` only picks the reading rules.
    #[tokio::test]
    #[ignore = "requires PTTS_TEST_MODEL and the Hub voice catalog"]
    async fn catalog_voices_of_each_language_in_one_instance() {
        let model = std::env::var("PTTS_TEST_MODEL").expect("set PTTS_TEST_MODEL");
        let app = load_ptts(
            &model,
            None,
            Some(ptts::voice_catalog::DEFAULT_VOICE_CATALOG),
            ptts::synth::DeviceKind::Cpu,
            ptts::synth::Quant::Q80,
            0.3,
            7,
            1024,
            None,
            Rules::DEFAULT,
            true,
            &[],
            std::num::NonZeroUsize::new(2).unwrap(),
        )
        .await
        .unwrap();
        let pt = "Temos R$ 10,50 e a Dra. Ana, em 2 de março, às 15h.";
        let en = "We have $10.50 and Dr. Smith, on March 2nd at 3pm.";
        let cases = [
            ("Débora", "pt", pt),
            ("Davi", "pt", pt),
            ("Manuela-Lu", "pt", pt),
            ("Easton", "en", en),
        ];
        for (voice, lang, text) in cases {
            let pcm = speak_as(&app, Some(lang), Some(voice), text).await.unwrap();
            assert!(!pcm.is_empty(), "{voice}");
            if let Ok(dir) = std::env::var("PTTS_TEST_WAV_DIR") {
                std::fs::create_dir_all(&dir).unwrap();
                let name = voice.to_lowercase();
                std::fs::write(format!("{dir}/{name}-{lang}.wav"), wav(&pcm, app.sample_rate))
                    .unwrap();
            }
        }
        let (code, message) = speak_as(&app, Some("en"), Some("Débora"), en).await.unwrap_err();
        assert_eq!(code, error_codes::VOICE_LANGUAGE_MISMATCH, "{message}");
        assert!(message.contains("does not speak"), "{message}");
        let (code, _) = speak_as(&app, Some("en"), Some("NoSuchVoice"), en).await.unwrap_err();
        assert_eq!(code, error_codes::NOT_FOUND);
    }
}
