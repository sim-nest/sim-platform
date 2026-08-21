//! Filesystem leaf path, read, and write helpers.

use sim_codec::{Input, Output, decode_with_codec, encode_with_codec};
use sim_kernel::{Cx, EncodeOptions, Error, Expr, ReadPolicy, Result, Symbol};

use super::{FsDir, port_error};
use crate::roadmap11::{decode_expr_for_ext, encode_expr_for_ext, known_exts};

impl FsDir {
    pub(super) fn segment(&self, name: &Symbol) -> Result<Vec<String>> {
        let segment = name.name.as_ref();
        // The shared predicate rejects empty/`.`/`..`/`/`/`\`; table-fs keeps the
        // additional, stricter `is_absolute()` guard on top of it.
        if !sim_table_core::is_legal_table_segment(segment)
            || std::path::Path::new(segment).is_absolute()
        {
            return Err(Error::Eval(format!("table/fs: illegal name {segment:?}")));
        }
        Ok(vec![segment.to_owned()])
    }

    pub(super) fn leaf_name(&self, name: &Symbol, ext: &str) -> Result<Vec<String>> {
        self.segment(name)?;
        self.segment(&Symbol::new(format!("{}.{}", name.name, ext)))
    }

    pub(super) fn leaf_candidates(
        &self,
        name: &Symbol,
    ) -> Result<Vec<(Vec<String>, &'static str)>> {
        let mut matches = Vec::new();
        for ext in known_exts() {
            let path = self.leaf_name(name, ext)?;
            if self.port.metadata(&path).map_err(port_error)?.is_some() {
                matches.push((path, ext));
            }
        }
        Ok(matches)
    }

    pub(super) fn leaf_path_for_read(
        &self,
        name: &Symbol,
    ) -> Result<Option<(Vec<String>, &'static str)>> {
        let matches = self.leaf_candidates(name)?;
        match matches.len() {
            0 => Ok(None),
            1 => Ok(matches.into_iter().next()),
            _ => Err(Error::Eval(format!(
                "table/fs: multiple leaf files found for key {name}"
            ))),
        }
    }

    pub(crate) fn read_leaf_expr(
        &self,
        cx: &mut Cx,
        key: &Symbol,
    ) -> Result<(Vec<String>, &'static str, Expr)> {
        let Some((path, ext)) = self.leaf_path_for_read(key)? else {
            return Err(Error::Eval(format!("table/fs: {key} is not a file")));
        };
        let expr = self.read_leaf_path(cx, &path, ext)?;
        Ok((path, ext, expr))
    }

    pub(crate) fn read_leaf_path(&self, cx: &mut Cx, path: &[String], ext: &str) -> Result<Expr> {
        let bytes = self.port.read(path).map_err(port_error)?;
        Ok(match decode_expr_for_ext(ext, &bytes) {
            Some(expr) => expr?,
            None => {
                let codec = Self::codec_for_ext(ext)?;
                Self::decode_expr_bytes(cx, &codec, &bytes)?
            }
        })
    }

    fn codec_for_ext(ext: &str) -> Result<Symbol> {
        match ext {
            "siml" => Ok(Symbol::qualified("codec", "lisp")),
            "simb" => Ok(Symbol::qualified("codec", "binary")),
            "simb64" => Ok(Symbol::qualified("codec", "binary-base64")),
            "simj" => Ok(Symbol::qualified("codec", "json")),
            "sima" => Ok(Symbol::qualified("codec", "algol")),
            other => Err(Error::Eval(format!("table/fs: unknown extension {other}"))),
        }
    }

    fn decode_expr_bytes(cx: &mut Cx, codec: &Symbol, bytes: &[u8]) -> Result<Expr> {
        decode_with_codec(
            cx,
            codec,
            Input::Bytes(bytes.to_vec()),
            ReadPolicy::default(),
        )
    }

    fn encode_expr_bytes(cx: &mut Cx, codec: &Symbol, expr: &Expr) -> Result<Vec<u8>> {
        match encode_with_codec(cx, codec, expr, EncodeOptions::default())? {
            Output::Text(text) => Ok(text.into_bytes()),
            Output::Bytes(bytes) => Ok(bytes),
        }
    }

    pub(crate) fn encode_leaf_expr(cx: &mut Cx, ext: &str, expr: &Expr) -> Result<Vec<u8>> {
        match encode_expr_for_ext(ext, expr) {
            Some(bytes) => bytes,
            None => {
                let codec = Symbol::qualified("codec", "lisp");
                Self::encode_expr_bytes(cx, &codec, expr)
            }
        }
    }
}
