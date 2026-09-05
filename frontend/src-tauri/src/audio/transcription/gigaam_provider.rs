// audio/transcription/gigaam_provider.rs
//
// GigaAM transcription provider implementation.

use super::provider::{TranscriptionError, TranscriptionProvider, TranscriptResult};
use async_trait::async_trait;
use log::warn;
use std::sync::Arc;

/// Shorter than this the encoder has almost nothing to subsample (0.2 s at 16 kHz).
const MIN_SAMPLES: usize = 3200;

/// GigaAM transcription provider (wraps GigaamEngine)
pub struct GigaamProvider {
    engine: Arc<crate::gigaam_engine::GigaamEngine>,
}

impl GigaamProvider {
    pub fn new(engine: Arc<crate::gigaam_engine::GigaamEngine>) -> Self {
        Self { engine }
    }
}

#[async_trait]
impl TranscriptionProvider for GigaamProvider {
    async fn transcribe(
        &self,
        audio: Vec<f32>,
        language: Option<String>,
    ) -> std::result::Result<TranscriptResult, TranscriptionError> {
        if audio.len() < MIN_SAMPLES {
            return Err(TranscriptionError::AudioTooShort {
                samples: audio.len(),
                minimum: MIN_SAMPLES,
            });
        }

        // GigaAM is a Russian-only model; the language hint has nothing to select
        if let Some(language) = language
            .as_deref()
            .filter(|value| !matches!(*value, "ru" | "auto" | "auto-translate" | ""))
        {
            warn!(
                "GigaAM transcribes Russian only - ignoring language preference '{}'",
                language
            );
        }

        match self.engine.transcribe_audio(audio).await {
            Ok(text) => Ok(TranscriptResult {
                text: text.trim().to_string(),
                confidence: None,  // Greedy transducer decoding reports no score
                is_partial: false, // Each chunk is decoded in full
            }),
            Err(e) => Err(TranscriptionError::EngineFailed(e.to_string())),
        }
    }

    async fn is_model_loaded(&self) -> bool {
        self.engine.is_model_loaded().await
    }

    async fn get_current_model(&self) -> Option<String> {
        self.engine.get_current_model().await
    }

    fn provider_name(&self) -> &'static str {
        "GigaAM"
    }
}
