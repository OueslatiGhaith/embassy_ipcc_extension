use crate::ipcc::Ipcc;

use super::{
    cmd::CmdPacket, consts::TlPacketType, sys, TL_CS_EVT_SIZE, TL_EVT_HEADER_SIZE,
    TL_PACKET_HEADER_SIZE, TL_SYS_TABLE,
};

const SCHI_OPCODE_BLE_INIT: u16 = 0xfc66;

#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct ShciBleInitCmdParam {
    /// NOT USED CURRENTLY
    pub p_ble_buffer_address: u32,

    /// Size of the Buffer allocated in pBleBufferAddress
    pub ble_buffer_size: u32,

    pub num_attr_record: u16,
    pub num_attr_serv: u16,
    pub attr_value_arr_size: u16,
    pub num_of_links: u8,
    pub extended_packet_length_enable: u8,
    pub pr_write_list_size: u8,
    pub mb_lock_count: u8,

    pub att_mtu: u16,
    pub slave_sca: u16,
    pub master_sca: u8,
    pub ls_source: u8,
    pub max_conn_event_length: u32,
    pub hs_startup_time: u16,
    pub viterbi_enable: u8,
    pub ll_only: u8,
    pub hw_version: u8,
}

#[derive(Debug, Clone, Copy, Default)]
#[repr(C, packed)]
pub struct ShciHeader {
    metadata: [u32; 3],
}

#[derive(Debug, Clone, Copy)]
#[repr(C, packed)]
pub struct ShciBleInitCmdPacket {
    header: ShciHeader,
    param: ShciBleInitCmdParam,
}

pub const TL_BLE_EVT_CS_PACKET_SIZE: usize = TL_EVT_HEADER_SIZE + TL_CS_EVT_SIZE;
#[allow(dead_code)] // Not used currently but reserved
const TL_BLE_EVT_CS_BUFFER_SIZE: usize = TL_PACKET_HEADER_SIZE + TL_BLE_EVT_CS_PACKET_SIZE;

pub fn shci_ble_init(ipcc: &mut Ipcc, param: ShciBleInitCmdParam) {
    defmt::debug!("sending shci init");

    let mut packet = ShciBleInitCmdPacket {
        header: ShciHeader::default(),
        param,
    };

    let packet_ptr: *mut _ = &mut packet;

    unsafe {
        let cmd_ptr: *mut CmdPacket = packet_ptr.cast();

        (*cmd_ptr).cmdserial.cmd.cmd_code = SCHI_OPCODE_BLE_INIT;
        (*cmd_ptr).cmdserial.cmd.payload_len = core::mem::size_of::<ShciBleInitCmdParam>() as u8;

        let mut p_cmd_buffer = &mut *(*TL_SYS_TABLE.as_mut_ptr()).pcmd_buffer;
        core::ptr::write(p_cmd_buffer, *cmd_ptr);

        p_cmd_buffer.cmdserial.ty = TlPacketType::SysCmd as u8;

        let a = unsafe {
            core::slice::from_raw_parts(
                p_cmd_buffer as *const _ as *const u8,
                core::mem::size_of::<CmdPacket>(),
            )
        };
        defmt::debug!("sending SHCI {:#04x}", a);

        sys::send_cmd(ipcc);
    }
}
