// ─── Error ──────────────────────────────────────────────────────────────────
#[derive(Debug)]
pub enum RecordError {
    SerializationNotObject,
    InvalidBuffer,
    TooManyFields,
    FieldNotFound,
    TypeMismatch { expected: u8, actual: u8 },
    LengthMismatch { expected: usize, actual: usize },
    FieldExists,
    CborError(String),
}

impl std::fmt::Display for RecordError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RecordError::SerializationNotObject => write!(f, "Can't Serialize none Object Types"),
            RecordError::InvalidBuffer => write!(f, "Invalid buffer structure"),
            RecordError::TooManyFields => write!(f, ">= 32 Entetys"),
            RecordError::FieldNotFound => write!(f, "Field not found"),
            RecordError::TypeMismatch { expected, actual } => {
                write!(f, "Type mismatch: expected {}, got {}", expected, actual)
            }
            RecordError::LengthMismatch { expected, actual } => {
                write!(f, "Length mismatch: expected {}, got {}", expected, actual)
            }
            RecordError::FieldExists => write!(f, "Field already exists"),
            RecordError::CborError(msg) => write!(f, "CBOR error: {}", msg),
        }
    }
}

impl std::error::Error for RecordError {}
