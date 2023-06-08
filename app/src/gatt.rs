use bluetooth_hci::{
    event::command::{CommandComplete, ReturnParameters},
    host::uart::Packet,
    Event,
};
use rf::hci::{
    command::gatt::{
        AddCharacteristicParameters, AddServiceParameters, CharacteristicEvent,
        CharacteristicHandle, CharacteristicPermission, CharacteristicProperty, EncryptionKeySize,
        GattCommands, ServiceHandle, ServiceType, Uuid,
    },
    event,
};

use crate::{helpers::receive_event_helper, Rc};

type CP = CharacteristicProperty;

pub struct BleContext {
    pub service_handle: ServiceHandle,
    // pub read_char_handle: CharacteristicHandle,
    // pub write_char_handle: CharacteristicHandle,
    // pub notify_char_handle: CharacteristicHandle,
}

pub async fn init_gatt_services(rc: &mut Rc) -> Result<BleContext, nb::Error<()>> {
    defmt::info!("initializing services and characteristics");

    let characteristics = [
        (Uuid::Uuid16(0x00FF), CP::READ | CP::WRITE),
        (Uuid::Uuid16(0x00FE), CP::NOTIFY | CP::WRITE),
        (Uuid::Uuid16(0x0605), CP::READ | CP::WRITE),
        (Uuid::Uuid16(0x0505), CP::NOTIFY | CP::READ),
        (Uuid::Uuid16(0x0405), CP::READ | CP::WRITE),
        (Uuid::Uuid16(0x0305), CP::NOTIFY | CP::READ),
        (Uuid::Uuid16(0x0205), CP::READ | CP::WRITE),
        (Uuid::Uuid16(0x0105), CP::NOTIFY | CP::READ),
    ];

    let service_handle = gatt_add_service(rc, Uuid::Uuid16(0x5445)).await?;

    for char in characteristics {
        gatt_add_char(rc, service_handle, char.0, char.1).await?;
    }

    // let read_char_handle =
    //     gatt_add_char(rc, service_handle, Uuid::Uuid16(0x501), CharacteristicProperty::READ)
    //         .await?;
    // let write_char_handle =
    //     gatt_add_char(rc, service_handle, Uuid::Uuid16(0x501), CharacteristicProperty::WRITE)
    //         .await?;
    // let notify_char_handle = gatt_add_char(
    //     rc,
    //     service_handle,
    //     Uuid::Uuid16(0x501),
    //     CharacteristicProperty::NOTIFY | CharacteristicProperty::READ,
    // )
    // .await?;

    Ok(BleContext {
        service_handle,
        // read_char_handle,
        // write_char_handle,
        // notify_char_handle,
    })
}

async fn gatt_add_service(rc: &mut Rc, uuid: Uuid) -> Result<ServiceHandle, nb::Error<()>> {
    rc.add_service(&AddServiceParameters {
        uuid,
        service_type: ServiceType::Primary,
        max_attribute_records: 32,
    })?;
    let result = receive_event_helper(rc, true).await?;

    if let Packet::Event(Event::CommandComplete(CommandComplete {
        return_params:
            ReturnParameters::Vendor(event::command::ReturnParameters::GattAddService(
                event::command::GattService { service_handle, .. },
            )),
        ..
    })) = result
    {
        Ok(service_handle)
    } else {
        Err(nb::Error::Other(()))
    }
}

async fn gatt_add_char(
    rc: &mut Rc,
    service_handle: ServiceHandle,
    characteristic_uuid: Uuid,
    characteristic_properties: CharacteristicProperty,
) -> Result<CharacteristicHandle, nb::Error<()>> {
    rc.add_characteristic(&AddCharacteristicParameters {
        service_handle,
        characteristic_uuid,
        characteristic_properties,
        characteristic_value_len: 16,
        security_permissions: CharacteristicPermission::empty(),
        gatt_event_mask: CharacteristicEvent::all(),
        encryption_key_size: EncryptionKeySize::with_value(7).unwrap(),
        is_variable: true,
        fw_version_before_v72: false,
    })?;
    let result = receive_event_helper(rc, true).await?;

    if let Packet::Event(Event::CommandComplete(CommandComplete {
        return_params:
            ReturnParameters::Vendor(event::command::ReturnParameters::GattAddCharacteristic(
                event::command::GattCharacteristic {
                    characteristic_handle,
                    ..
                },
            )),
        ..
    })) = result
    {
        Ok(characteristic_handle)
    } else {
        Err(nb::Error::Other(()))
    }
}
