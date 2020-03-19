use std::io::{Cursor, Read};

use super::*;
use crate::compression::Compressor;
use crate::error;
use crate::frame::frame_response::ResponseBody;
use crate::frame::FromCursor;
use crate::types::data_serialization_types::decode_timeuuid;
use crate::types::{from_bytes, from_u16_bytes, CStringList, UUID_LEN};

macro_rules! proceed_if_filled {
  ($res: expr, $expected_len: expr) => {
    if $expected_len > $res? {
      return Ok(None);
    }
  };
}

pub fn parse_frame_async<E, C>(
  cursor: &mut C,
  compressor: &dyn Compressor<CompressorError = E>,
) -> error::Result<Option<Frame>>
where
  E: std::error::Error,
  C: Read,
{
  let mut version_bytes = [0; Version::BYTE_LENGTH];
  let mut flag_bytes = [0; Flag::BYTE_LENGTH];
  let mut opcode_bytes = [0; Opcode::BYTE_LENGTH];
  let mut stream_bytes = [0; STREAM_LEN];
  let mut length_bytes = [0; LENGTH_LEN];

  // NOTE: order of reads matters
  proceed_if_filled!(cursor.read(&mut version_bytes), Version::BYTE_LENGTH);
  println!("version {:?}", version_bytes);
  proceed_if_filled!(cursor.read(&mut flag_bytes), Flag::BYTE_LENGTH);
  println!("flag {:?}", flag_bytes);
  proceed_if_filled!(cursor.read(&mut stream_bytes), STREAM_LEN);
  println!("stream {:?}", stream_bytes);
  proceed_if_filled!(cursor.read(&mut opcode_bytes), Opcode::BYTE_LENGTH);
  println!("opcode {:?}", opcode_bytes);
  proceed_if_filled!(cursor.read(&mut length_bytes), LENGTH_LEN);
  println!("length {:?}", length_bytes);

  let version = Version::from(version_bytes.to_vec());
  let flags = Flag::get_collection(flag_bytes[0]);
  let stream = from_u16_bytes(&stream_bytes);
  let opcode = Opcode::from(opcode_bytes[0]);
  let length = from_bytes(&length_bytes) as usize;

  let mut body_bytes = Vec::with_capacity(length);
  unsafe {
    body_bytes.set_len(length);
  }

  proceed_if_filled!(cursor.read(&mut body_bytes), length);

  let full_body = if flags.iter().any(|flag| flag == &Flag::Compression) {
    compressor
      .decode(body_bytes)
      .map_err(|err| error::Error::from(err.description()))?
  } else {
    body_bytes
  };

  // Use cursor to get tracing id, warnings and actual body
  let mut body_cursor = Cursor::new(full_body.as_slice());

  let tracing_id = if flags.iter().any(|flag| flag == &Flag::Tracing) {
    let mut tracing_bytes = Vec::with_capacity(UUID_LEN);
    unsafe {
      tracing_bytes.set_len(UUID_LEN);
    }
    body_cursor.read_exact(&mut tracing_bytes)?;

    decode_timeuuid(tracing_bytes.as_slice()).ok()
  } else {
    None
  };

  let warnings = if flags.iter().any(|flag| flag == &Flag::Warning) {
    CStringList::from_cursor(&mut body_cursor)?.into_plain()
  } else {
    vec![]
  };

  let mut body = vec![];

  body_cursor.read_to_end(&mut body)?;

  let frame = Frame {
    version: version,
    flags: flags,
    opcode: opcode,
    stream: stream,
    body: body,
    tracing_id: tracing_id,
    warnings: warnings,
  };

  Ok(Some(frame))

  // convert_frame_into_result(frame).map(Option::Some)
}

pub fn convert_frame_into_result(frame: Frame) -> error::Result<Frame> {
  match frame.opcode {
    Opcode::Error => frame.get_body().and_then(|err| match err {
      ResponseBody::Error(err) => Err(error::Error::Server(err)),
      _ => unreachable!(),
    }),
    _ => Ok(frame),
  }
}
