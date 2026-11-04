//! Rockusb wire protocol: command block / status wrappers and the opcode table.
//!
//! Every constant here was recovered from Rockchip `upgrade_tool` v2.17
//! (x86-64 ELF, not stripped) by disassembling
//! `CRKUsbComm::InitializeCBW(CBW*, USB_OPERATION_CODE)` at `0x4477d0`.
//! The direction / CDB-length pairs in [`Opcode::shape`] are a direct decode of
//! that function's jump table and bitmask tests, so they are observed
//! behaviour rather than inference.

/// Rockchip's USB vendor ID. Shared by MaskROM and Loader modes.
pub const ROCKCHIP_VID: u16 = 0x2207;

/// `CBW` signature, "USBC". Written by the tool as the LE word `0x43425355`.
pub const CBW_SIGNATURE: [u8; 4] = *b"USBC";
/// `CSW` signature, "USBS".
pub const CSW_SIGNATURE: [u8; 4] = *b"USBS";

/// Command Block Wrapper length on the wire.
pub const CBW_LEN: usize = 31;
/// Command Status Wrapper length on the wire.
pub const CSW_LEN: usize = 13;
/// Flash sector size used by every LBA-addressed command.
pub const SECTOR_SIZE: usize = 512;

/// Direction bit stored at `CBW` offset 12.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Device to host.
    In = 0x80,
    /// Host to device.
    Out = 0x00,
}

/// Rockusb command opcodes, stored at `CBW` offset 15 (`CDB[0]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Opcode {
    TestUnitReady = 0x00,
    ReadFlashId = 0x01,
    TestBadBlock = 0x03,
    ReadSector = 0x04,
    WriteSector = 0x05,
    EraseNormal = 0x06,
    EraseForce = 0x0b,
    ReadLba = 0x14,
    WriteLba = 0x15,
    EraseSystemDisk = 0x16,
    ReadSdram = 0x17,
    WriteSdram = 0x18,
    ExecuteSdram = 0x19,
    ReadFlashInfo = 0x1a,
    ReadChipInfo = 0x1b,
    SetResetFlag = 0x1e,
    WriteEfuse = 0x1f,
    ReadEfuse = 0x20,
    ReadSpiFlash = 0x21,
    WriteSpiFlash = 0x22,
    WriteNewEfuse = 0x23,
    ReadNewEfuse = 0x24,
    EraseLba = 0x25,
    /// Present in the dispatch table (IN, CDB10) but unnamed in the tool.
    ReadCapacity28 = 0x28,
    SwitchStorage = 0x2a,
    GetStorageMedia = 0x2b,
    ReadCapability = 0xaa,
    DeviceReset = 0xff,
}

impl Opcode {
    /// Direction and CDB length for this opcode.
    ///
    /// Decoded from `InitializeCBW`: the function zeroes the wrapper, writes the
    /// opcode to offset 15, then selects one of four `(flags, cdb_len)` pairs.
    /// Opcodes `0x02`, `0x1c` and `0x1d` fall through without either field
    /// being set, so they are not represented in [`Opcode`] at all.
    pub const fn shape(self) -> (Direction, u8) {
        use Direction::{In, Out};
        match self {
            // IN, CDB6 -- 0x4478ca
            Opcode::TestUnitReady
            | Opcode::ReadFlashId
            | Opcode::ReadFlashInfo
            | Opcode::ReadChipInfo
            | Opcode::ReadEfuse
            | Opcode::GetStorageMedia
            | Opcode::ReadCapability => (In, 6),
            // IN, CDB10 -- 0x447938
            Opcode::TestBadBlock
            | Opcode::ReadSector
            | Opcode::ReadLba
            | Opcode::ReadSdram
            | Opcode::ReadSpiFlash
            | Opcode::ReadNewEfuse
            | Opcode::ReadCapacity28 => (In, 10),
            // OUT, CDB6 -- 0x4478af
            Opcode::EraseSystemDisk
            | Opcode::SetResetFlag
            | Opcode::SwitchStorage
            | Opcode::DeviceReset => (Out, 6),
            // OUT, CDB10 -- 0x4478fa
            Opcode::WriteSector
            | Opcode::EraseNormal
            | Opcode::EraseForce
            | Opcode::WriteLba
            | Opcode::WriteSdram
            | Opcode::ExecuteSdram
            | Opcode::WriteEfuse
            | Opcode::WriteSpiFlash
            | Opcode::WriteNewEfuse
            | Opcode::EraseLba => (Out, 10),
        }
    }

    /// Direction bit this opcode is issued with.
    pub const fn direction(self) -> Direction {
        self.shape().0
    }

    /// CDB length this opcode is issued with (6 or 10).
    pub const fn cdb_len(self) -> u8 {
        self.shape().1
    }
}

/// Sub-opcode for [`Opcode::DeviceReset`], stored at `CBW` offset 16.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ResetOpcode {
    /// Reset the device and re-enumerate.
    Reset = 0,
    /// Switch to USB mass storage.
    MassStorage = 1,
    /// Power the device off.
    PowerOff = 2,
    /// Drop back into MaskROM. This is `upgrade_tool rd 3`.
    Maskrom = 3,
    /// Disconnect without resetting.
    Disconnect = 4,
}

/// Which storage device subsequent commands address.
///
/// Values match the bitmask returned by [`Opcode::GetStorageMedia`] and
/// accepted by [`Opcode::SwitchStorage`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum StorageMedia {
    Flash = 0,
    Emmc = 1,
    SpiNor = 2,
    SpiNand = 3,
    Sd = 4,
    UsbDisk = 5,
    Sata = 6,
    Pcie = 7,
}

impl StorageMedia {
    /// Decode the one-based code used by `upgrade_tool ssd` and the
    /// `GetStorageMedia` bitmask.
    pub const fn from_code(code: u8) -> Option<Self> {
        Some(match code {
            0 => Self::Flash,
            1 => Self::Emmc,
            2 => Self::SpiNor,
            3 => Self::SpiNand,
            4 => Self::Sd,
            5 => Self::UsbDisk,
            6 => Self::Sata,
            7 => Self::Pcie,
            _ => return None,
        })
    }

    /// Human-readable name, matching the tool's `UL` storage arguments.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Flash => "FLASH",
            Self::Emmc => "EMMC",
            Self::SpiNor => "SPINOR",
            Self::SpiNand => "SPINAND",
            Self::Sd => "SD",
            Self::UsbDisk => "USB",
            Self::Sata => "SATA",
            Self::Pcie => "PCIE",
        }
    }

    /// Decode the storage bitmask reported by `GetStorageMedia`.
    pub fn from_mask(mask: u32) -> Vec<Self> {
        (0..8)
            .filter(|bit| mask & (1 << bit) != 0)
            .filter_map(|bit| Self::from_code(bit as u8))
            .collect()
    }
}

/// A Command Block Wrapper, laid out exactly as `InitializeCBW` builds it.
///
/// ```text
/// 0..4   signature "USBC"
/// 4..8   tag                      (big endian)
/// 8..12  transfer length          (little endian)
/// 12     flags: 0x80 IN / 0x00 OUT
/// 13     LUN (always 0)
/// 14     CDB length (6 or 10)
/// 15     CDB[0] opcode
/// 16     CDB[1] sub-opcode
/// 17..21 CDB[2..6] address        (big endian)
/// 21     CDB[6] reserved
/// 22..24 CDB[7..9] length         (big endian)
/// 24..31 zero padding
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cbw {
    pub tag: u32,
    pub transfer_len: u32,
    pub opcode: Opcode,
    pub sub_opcode: u8,
    pub address: u32,
    pub length: u16,
}

impl Cbw {
    /// Build a wrapper for `opcode`, deriving direction and CDB length from it.
    pub fn new(tag: u32, opcode: Opcode) -> Self {
        Self {
            tag,
            transfer_len: 0,
            opcode,
            sub_opcode: 0,
            address: 0,
            length: 0,
        }
    }

    /// Set the host/device payload size for the data phase.
    pub fn with_transfer_len(mut self, len: u32) -> Self {
        self.transfer_len = len;
        self
    }

    /// Set `CDB[1]`, used by `DeviceReset` and the LBA commands' flag byte.
    pub fn with_sub_opcode(mut self, sub: u8) -> Self {
        self.sub_opcode = sub;
        self
    }

    /// Set the big-endian address at `CDB[2..6]` (an LBA for most commands).
    pub fn with_address(mut self, address: u32) -> Self {
        self.address = address;
        self
    }

    /// Set the big-endian count at `CDB[7..9]` (sectors, for LBA commands).
    pub fn with_length(mut self, length: u16) -> Self {
        self.length = length;
        self
    }

    /// Direction of this command's data phase.
    pub fn direction(&self) -> Direction {
        self.opcode.direction()
    }

    /// Serialise to the 31 bytes sent on the bulk OUT endpoint.
    pub fn to_bytes(self) -> [u8; CBW_LEN] {
        let mut b = [0u8; CBW_LEN];
        b[0..4].copy_from_slice(&CBW_SIGNATURE);
        b[4..8].copy_from_slice(&self.tag.to_be_bytes());
        b[8..12].copy_from_slice(&self.transfer_len.to_le_bytes());
        let (dir, cdb_len) = self.opcode.shape();
        b[12] = dir as u8;
        b[13] = 0; // LUN
        b[14] = cdb_len;
        b[15] = self.opcode as u8;
        b[16] = self.sub_opcode;
        b[17..21].copy_from_slice(&self.address.to_be_bytes());
        b[21] = 0;
        b[22..24].copy_from_slice(&self.length.to_be_bytes());
        b
    }
}

/// Status returned by the device in the `CSW`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CswStatus {
    Success,
    Failed,
    /// Any value the tool does not recognise.
    Other(u8),
}

/// A Command Status Wrapper, read from the bulk IN endpoint after the data phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Csw {
    pub tag: u32,
    pub residue: u32,
    pub status: CswStatus,
}

impl Csw {
    /// Parse the 13 bytes returned on the bulk IN endpoint.
    pub fn from_bytes(b: &[u8]) -> Result<Self, ProtocolError> {
        if b.len() < CSW_LEN {
            return Err(ProtocolError::ShortCsw(b.len()));
        }
        if b[0..4] != CSW_SIGNATURE {
            let mut sig = [0u8; 4];
            sig.copy_from_slice(&b[0..4]);
            return Err(ProtocolError::BadCswSignature(sig));
        }
        Ok(Self {
            tag: u32::from_be_bytes([b[4], b[5], b[6], b[7]]),
            residue: u32::from_le_bytes([b[8], b[9], b[10], b[11]]),
            status: match b[12] {
                0 => CswStatus::Success,
                1 => CswStatus::Failed,
                other => CswStatus::Other(other),
            },
        })
    }

    /// True when the device reported success.
    pub fn is_success(&self) -> bool {
        self.status == CswStatus::Success
    }
}

/// MaskROM bootstrap request codes, issued as USB control transfers.
///
/// `CRKUsbComm::RKU_DeviceRequest` range-checks its request argument with
/// `leal -0x471(%rsi); cmpl $1` then `ja`, so only these two are accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum BootStage {
    /// First stage: DDR init blob (`471` entries in the loader).
    Ddr = 0x471,
    /// Second stage: the USB loader itself (`472` entries).
    Loader = 0x472,
}

/// Maximum payload per 471/472 control transfer, before the CRC is appended.
pub const BOOT_CHUNK_SIZE: usize = 4096;

/// Errors raised while framing or parsing the protocol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolError {
    /// Fewer than 13 bytes came back for the status wrapper.
    ShortCsw(usize),
    /// The status wrapper did not start with "USBS".
    BadCswSignature([u8; 4]),
    /// The device echoed a tag that does not match the command.
    TagMismatch { sent: u32, got: u32 },
    /// The device reported a failure status.
    CommandFailed { opcode: u8, status: CswStatus },
}

impl core::fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ShortCsw(n) => write!(f, "short CSW: expected {CSW_LEN} bytes, got {n}"),
            Self::BadCswSignature(sig) => {
                write!(f, "bad CSW signature: expected \"USBS\", got {sig:02x?}")
            }
            Self::TagMismatch { sent, got } => {
                write!(f, "CSW tag mismatch: sent {sent:#010x}, got {got:#010x}")
            }
            Self::CommandFailed { opcode, status } => {
                write!(f, "command {opcode:#04x} failed: {status:?}")
            }
        }
    }
}

impl std::error::Error for ProtocolError {}
