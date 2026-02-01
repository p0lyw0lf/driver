use koto::bytecode::Instruction;
use koto::bytecode::InstructionReader;
use koto::runtime::KFunction;
use sha2::Digest;

use crate::to_hash::Hash;
use crate::to_hash::ToHash;

/// Newtype that implements PartialEq based on the hash of the inner function.
#[derive(Clone)]
pub enum HashedKFunction {
    /// Stored at runtime
    Function(KFunction),
    /// Loaded from disk. Only used for checking that new functions, if they needed to be
    /// re-generated, are the same as the last time we ran.
    Hash(Hash),
}

impl ToHash for HashedKFunction {
    fn to_hash(&self) -> Hash {
        match self {
            HashedKFunction::Function(kfunction) => kfunction.to_hash(),
            HashedKFunction::Hash(hash) => *hash,
        }
    }

    fn run_hash(&self, hasher: &mut sha2::Sha256) {
        match self {
            HashedKFunction::Function(kfunction) => kfunction.run_hash(hasher),
            // NOTE: this is different behavior from to_hash(), on purpose. It's probably fine.
            // I think.
            HashedKFunction::Hash(hash) => hasher.update(hash),
        }
    }
}

impl PartialEq for HashedKFunction {
    fn eq(&self, other: &Self) -> bool {
        self.to_hash() == other.to_hash()
    }
}
impl Eq for HashedKFunction {}

impl std::fmt::Debug for HashedKFunction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HashedKFunction::Function(_) => write!(f, "Function"),
            HashedKFunction::Hash(hash) => write!(f, "Hash({hash:?})"),
        }
    }
}

impl std::hash::Hash for HashedKFunction {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // We want both cases to be hashed to the same thing
        // core::mem::discriminant(self).hash(state);
        self.to_hash().hash(state);
    }
}

impl ToHash for KFunction {
    fn run_hash(&self, hasher: &mut sha2::Sha256) {
        hasher.update(b"KFunction");
        hasher.update(self.ip.to_le_bytes());
        hasher.update(self.arg_count.to_le_bytes());
        hasher.update(self.optional_arg_count.to_le_bytes());
        self.flags.run_hash(hasher);
        if self.context.is_some() {
            todo!("Cannot hash KFunctions that include context");
        }
        // In order for the hash of the function to remain the same given different constant pools,
        // we need to incorporate the _value of the constant_ when doing the hash of the bytes.
        macro_rules! fields {
            ( $instruction:expr => $( $case:ident $( { $(
                $(bytes $b_name:ident)?
                $(hash $h_name:ident)?
                $(constant $constant:ident)?
                ,
            )+ } )? , )* ) => {
                use Instruction::*;
                match $instruction { $(
                    $case $( { $(
                        $($b_name,)?
                        $($h_name,)?
                        $($constant,)?
                    )+ } )? => {
                        hasher.update(stringify!($case));
                    $($(
                        $(hasher.update($b_name.to_le_bytes());)?
                        $($h_name.run_hash(hasher);)?
                        $(self.chunk.constants.get(usize::from($constant)).expect("invalid constant index").run_hash(hasher);)?
                    )+)? }
                )* }
            }
        }
        let instruction_reader = InstructionReader::new(self.chunk.clone());
        for instruction in instruction_reader.into_iter() {
            fields!(instruction =>
                Error {
                    hash message,
                },
                NewFrame {
                    bytes register_count,
                },
                Copy {
                    bytes target,
                    bytes source,
                },
                SetNull {
                    bytes register,
                },
                SetBool {
                    bytes register,
                    hash value,
                },
                SetNumber {
                    bytes register,
                    bytes value,
                },
                LoadFloat {
                    bytes register,
                    constant constant,
                },
                LoadInt {
                    bytes register,
                    constant constant,
                },
                LoadString {
                    bytes register,
                    constant constant,
                },
                LoadNonLocal {
                    bytes register,
                    constant constant,
                },
                ExportValue {
                    bytes key,
                    bytes value,
                },
                ExportEntry {
                    bytes entry,
                },
                Import {
                    bytes register,
                },
                ImportAll {
                    bytes register,
                },
                MakeTempTuple {
                    bytes register,
                    bytes start,
                    bytes count,
                },
                TempTupleToTuple {
                    bytes register,
                    bytes source,
                },
                MakeMap {
                    bytes register,
                    bytes size_hint,
                },
                SequenceStart {
                    bytes size_hint,
                },
                SequencePush {
                    bytes value,
                },
                SequencePushN {
                    bytes start,
                    bytes count,
                },
                SequenceToList {
                    bytes register,
                },
                SequenceToTuple {
                    bytes register,
                },
                Range {
                    bytes register,
                    bytes start,
                    bytes end,
                },
                RangeInclusive {
                    bytes register,
                    bytes start,
                    bytes end,
                },
                RangeTo {
                    bytes register,
                    bytes end,
                },
                RangeToInclusive {
                    bytes register,
                    bytes end,
                },
                RangeFrom {
                    bytes register,
                    bytes start,
                },
                RangeFull {
                    bytes register,
                },
                MakeIterator {
                    bytes register,
                    bytes iterable,
                },
                Function {
                    bytes register,
                    bytes arg_count,
                    bytes optional_arg_count,
                    bytes capture_count,
                    hash flags,
                    bytes size,
                },
                Capture {
                    bytes function,
                    bytes target,
                    bytes source,
                },
                Negate {
                    bytes register,
                    bytes value,
                },
                Not {
                    bytes register,
                    bytes value,
                },
                Add {
                    bytes register,
                    bytes lhs,
                    bytes rhs,
                },
                Subtract {
                    bytes register,
                    bytes lhs,
                    bytes rhs,
                },
                Multiply {
                    bytes register,
                    bytes lhs,
                    bytes rhs,
                },
                Divide {
                    bytes register,
                    bytes lhs,
                    bytes rhs,
                },
                Remainder {
                    bytes register,
                    bytes lhs,
                    bytes rhs,
                },
                Power {
                    bytes register,
                    bytes lhs,
                    bytes rhs,
                },
                AddAssign {
                    bytes lhs,
                    bytes rhs,
                },
                SubtractAssign {
                    bytes lhs,
                    bytes rhs,
                },
                MultiplyAssign {
                    bytes lhs,
                    bytes rhs,
                },
                DivideAssign {
                    bytes lhs,
                    bytes rhs,
                },
                RemainderAssign {
                    bytes lhs,
                    bytes rhs,
                },
                PowerAssign {
                    bytes lhs,
                    bytes rhs,
                },
                Less {
                    bytes register,
                    bytes lhs,
                    bytes rhs,
                },
                LessOrEqual {
                    bytes register,
                    bytes lhs,
                    bytes rhs,
                },
                Greater {
                    bytes register,
                    bytes lhs,
                    bytes rhs,
                },
                GreaterOrEqual {
                    bytes register,
                    bytes lhs,
                    bytes rhs,
                },
                Equal {
                    bytes register,
                    bytes lhs,
                    bytes rhs,
                },
                NotEqual {
                    bytes register,
                    bytes lhs,
                    bytes rhs,
                },
                Jump {
                    bytes offset,
                },
                JumpBack {
                    bytes offset,
                },
                JumpIfTrue {
                    bytes register,
                    bytes offset,
                },
                JumpIfFalse {
                    bytes register,
                    bytes offset,
                },
                JumpIfNull {
                    bytes register,
                    bytes offset,
                },
                Call {
                    bytes result,
                    bytes function,
                    bytes frame_base,
                    bytes arg_count,
                    bytes packed_arg_count,
                },
                CallInstance {
                    bytes result,
                    bytes function,
                    bytes instance,
                    bytes frame_base,
                    bytes arg_count,
                    bytes packed_arg_count,
                },
                Return {
                    bytes register,
                },
                Yield {
                    bytes register,
                },
                Throw {
                    bytes register,
                },
                Size {
                    bytes register,
                    bytes value,
                },
                IterNext {
                    hash result,
                    bytes iterator,
                    bytes jump_offset,
                    hash temporary_output,
                },
                TempIndex {
                    bytes register,
                    bytes value,
                    bytes index,
                },
                SliceFrom {
                    bytes register,
                    bytes value,
                    bytes index,
                },
                SliceTo {
                    bytes register,
                    bytes value,
                    bytes index,
                },
                Index {
                    bytes register,
                    bytes value,
                    bytes index,
                },
                IndexMut {
                    bytes register,
                    bytes index,
                    bytes value,
                },
                MapInsert {
                    bytes register,
                    bytes key,
                    bytes value,
                },
                MetaInsert {
                    bytes register,
                    bytes value,
                    hash id,
                },
                MetaInsertNamed {
                    bytes register,
                    bytes value,
                    hash id,
                    bytes name,
                },
                MetaExport {
                    hash id,
                    bytes value,
                },
                MetaExportNamed {
                    hash id,
                    bytes name,
                    bytes value,
                },
                Access {
                    bytes register,
                    bytes value,
                    constant key,
                },
                AccessString {
                    bytes register,
                    bytes value,
                    bytes key,
                },
                TryStart {
                    bytes arg_register,
                    bytes catch_offset,
                },
                TryEnd,
                Debug {
                    bytes register,
                    constant constant,
                },
                CheckSizeEqual {
                    bytes register,
                    bytes size,
                },
                CheckSizeMin {
                    bytes register,
                    bytes size,
                },
                AssertType {
                    bytes value,
                    hash allow_null,
                    constant type_string,
                },
                CheckType {
                    bytes value,
                    hash allow_null,
                    constant type_string,
                    bytes jump_offset,
                },
                StringStart {
                    bytes size_hint,
                },
                StringPush {
                    bytes value,
                    hash format_options,
                },
                StringFinish {
                    bytes register,
                },
            );
        }
    }
}

impl<'a> ToHash for koto::parser::Constant<'a> {
    fn run_hash(&self, hasher: &mut sha2::Sha256) {
        match self {
            koto::parser::Constant::F64(f) => {
                hasher.update(b"Constant::F64");
                hasher.update(f.to_le_bytes());
            }
            koto::parser::Constant::I64(i) => {
                hasher.update(b"Constant::I64");
                hasher.update(i.to_le_bytes());
            }
            koto::parser::Constant::Str(s) => {
                hasher.update(b"Constant::Str");
                hasher.update(s.as_bytes());
            }
        }
    }
}

impl ToHash for koto::bytecode::FunctionFlags {
    fn run_hash(&self, hasher: &mut sha2::Sha256) {
        // Will never be used standalone
        hasher.update([u8::from(*self)]);
    }
}

impl ToHash for bool {
    fn run_hash(&self, hasher: &mut sha2::Sha256) {
        hasher.update(b"bool");
        hasher.update([u8::from(*self)]);
    }
}

impl ToHash for koto::parser::MetaKeyId {
    fn run_hash(&self, hasher: &mut sha2::Sha256) {
        // Will never be used standalone
        hasher.update([*self as u8]);
    }
}

impl ToHash for Option<u8> {
    fn run_hash(&self, hasher: &mut sha2::Sha256) {
        match *self {
            None => hasher.update(b"u8::None"),
            Some(b) => {
                hasher.update(b"u8::Some");
                hasher.update([b]);
            }
        }
    }
}

impl ToHash for Option<u32> {
    fn run_hash(&self, hasher: &mut sha2::Sha256) {
        match *self {
            None => hasher.update(b"u32::None"),
            Some(b) => {
                hasher.update(b"u32::Some");
                hasher.update(b.to_le_bytes());
            }
        }
    }
}

impl ToHash for Option<koto::parser::StringFormatOptions> {
    fn run_hash(&self, hasher: &mut sha2::Sha256) {
        match self {
            None => hasher.update(b"StringFormatOptions::None"),
            Some(opts) => {
                hasher.update(b"StringFormatOptions::Some");
                opts.alignment.run_hash(hasher);
                opts.min_width.run_hash(hasher);
                opts.precision.run_hash(hasher);
                if opts.fill_character.is_some() {
                    // I should probably fix this...
                    todo!("can't hash fill character for Reasons");
                }
                opts.representation.run_hash(hasher);
            }
        }
    }
}

impl ToHash for koto::parser::StringAlignment {
    fn run_hash(&self, hasher: &mut sha2::Sha256) {
        // Will never be used standalone
        hasher.update([*self as u8]);
    }
}

impl ToHash for Option<koto::parser::StringFormatRepresentation> {
    fn run_hash(&self, hasher: &mut sha2::Sha256) {
        match self {
            None => hasher.update(b"StringFormatRepresentation::None"),
            Some(r) => {
                hasher.update(b"StringFormatRepresentation::Some");
                hasher.update([*r as u8]);
            }
        }
    }
}
