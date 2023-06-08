/// enables or disables access to the backup domain
pub fn set_backup_access(enabled: bool) {
    let pwr = embassy_stm32::pac::PWR;

    // ST: write twice the value to flash the APB-AHB bridge to ensure
    // the bit is written
    unsafe {
        pwr.cr1().modify(|w| w.set_dbp(enabled));
        pwr.cr1().modify(|w| w.set_dbp(enabled));
    }
}
