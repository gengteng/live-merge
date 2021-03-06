#![allow(dead_code)]
use bytes::{BufMut, Bytes, BytesMut};

pub enum H264Data {
    Configuration {
        raw: Bytes,
        record: AVCDecoderConfigurationRecord,
    },
    Data {
        timestamp: u32,
        data: Bytes,
    },
}

impl H264Data {
    pub fn configuration(raw: Bytes, record: AVCDecoderConfigurationRecord) -> Self {
        Self::Configuration { raw, record }
    }
    pub fn data(timestamp: u32, data: Bytes) -> Self {
        Self::Data { timestamp, data }
    }
}

impl From<H264Data> for Bytes {
    fn from(p: H264Data) -> Self {
        match p {
            H264Data::Configuration { record, .. } => {
                let mut buffer = BytesMut::new();
                record.write_to(&mut buffer);
                buffer.freeze()
            }
            H264Data::Data { data, .. } => data,
        }
    }
}

//aligned(8) class AVCDecoderConfigurationRecord {
// unsigned int(8) configurationVersion = 1;
// unsigned int(8) AVCProfileIndication;
// unsigned int(8) profile_compatibility;
// unsigned int(8) AVCLevelIndication;
// bit(6) reserved = ?111111?b;
// unsigned int(2) lengthSizeMinusOne;
// bit(3) reserved = ?111?b;
// unsigned int(5) numOfSequenceParameterSets;
// for (i=0; i< numOfSequenceParameterSets; i++) {
// unsigned int(16) sequenceParameterSetLength ;
// bit(8*sequenceParameterSetLength) sequenceParameterSetNALUnit;
// }
// unsigned int(8) numOfPictureParameterSets;
// for (i=0; i< numOfPictureParameterSets; i++) {
// unsigned int(16) pictureParameterSetLength;
// bit(8*pictureParameterSetLength) pictureParameterSetNALUnit;
// }
// }

#[derive(Debug, Clone)]
pub struct AVCDecoderConfigurationRecord {
    pub configuration_version: u8,
    pub profile_indication: u8,
    pub profile_compatibility: u8,
    pub level_indication: u8,
    // 6 bits reserved
    pub length_size_minus_one: u8, // u2
    // 3 bits reserved,
    pub num_of_sequence_parameter_sets: u8, // u5
    pub sequence_parameter_sets: Vec<(u16, Vec<u8>)>,
    pub num_of_picture_parameter_sets: u8,
    pub picture_parameter_sets: Vec<(u16, Vec<u8>)>,
}

impl AVCDecoderConfigurationRecord {
    pub fn new(profile_indication: u8, level_indication: u8) -> Self {
        Self {
            configuration_version: 1,
            profile_indication,
            profile_compatibility: 0,
            level_indication,
            length_size_minus_one: 0b11,
            num_of_sequence_parameter_sets: 0,
            sequence_parameter_sets: vec![],
            num_of_picture_parameter_sets: 0,
            picture_parameter_sets: vec![],
        }
    }

    pub fn add_sps(&mut self, data: Vec<u8>) {
        self.num_of_sequence_parameter_sets += 1;
        self.sequence_parameter_sets.push((data.len() as u16, data));
    }

    pub fn add_pps(&mut self, data: Vec<u8>) {
        self.num_of_picture_parameter_sets += 1;
        self.picture_parameter_sets.push((data.len() as u16, data))
    }

    pub fn write_to<B: BufMut>(&self, mut buffer: B) {
        buffer.put_u8(self.configuration_version);
        buffer.put_u8(self.profile_indication);
        buffer.put_u8(self.profile_compatibility);
        buffer.put_u8(self.level_indication);
        buffer.put_u8(0b11111100u8 | self.length_size_minus_one);

        buffer.put_u8(0b11100000u8 | self.num_of_sequence_parameter_sets);
        for (len, sps) in &self.sequence_parameter_sets {
            buffer.put_u16(*len);
            buffer.put_slice(sps.as_slice());
        }

        buffer.put_u8(self.num_of_picture_parameter_sets);
        for (len, pps) in &self.picture_parameter_sets {
            buffer.put_u16(*len);
            buffer.put_slice(pps.as_slice());
        }
    }
}
