// gigaam_engine
//
// Native GigaAM v3 transcription: log-mel features, ONNX Runtime sessions for
// the RNN-T encoder/decoder/joint, and greedy transducer decoding.

pub mod commands;
pub mod engine;
pub mod features;
pub mod model;

pub use engine::{GigaamEngine, GigaamModelStatus, MODEL_NAME};
pub use model::GigaamModel;
