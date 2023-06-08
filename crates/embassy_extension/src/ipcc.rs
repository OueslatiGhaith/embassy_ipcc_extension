use self::sealed::Instance;
use embassy_stm32::{
    into_ref, peripherals::IPCC, rcc::low_level::RccPeripheral, Peripheral, PeripheralRef,
};
use embassy_sync::waitqueue::AtomicWaker;

#[non_exhaustive]
#[derive(Clone, Copy, Default)]
pub struct Config {}

pub struct State {
    _waker: AtomicWaker,
}

impl State {
    pub(crate) const fn new() -> Self {
        Self {
            _waker: AtomicWaker::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, defmt::Format)]
#[repr(C)]
pub enum IpccChannel {
    Channel1 = 0x00000001,
    Channel2 = 0x00000002,
    Channel3 = 0x00000004,
    Channel4 = 0x00000008,
    Channel5 = 0x00000010,
    Channel6 = 0x00000020,
}

impl From<IpccChannel> for usize {
    fn from(value: IpccChannel) -> Self {
        match value {
            IpccChannel::Channel1 => 0,
            IpccChannel::Channel2 => 1,
            IpccChannel::Channel3 => 2,
            IpccChannel::Channel4 => 3,
            IpccChannel::Channel5 => 4,
            IpccChannel::Channel6 => 5,
        }
    }
}

pub(crate) mod sealed {
    use super::*;

    pub trait Instance: embassy_stm32::rcc::RccPeripheral {
        fn regs() -> embassy_stm32::pac::ipcc::Ipcc;
        fn set_cpu2(enabled: bool);
        // fn clock(config: Config);
        fn state() -> &'static State;
    }
}

pub struct Ipcc<'d> {
    _peri: PeripheralRef<'d, IPCC>,
}

impl<'d> Ipcc<'d> {
    pub fn new(peri: impl Peripheral<P = IPCC> + 'd, _config: Config) -> Self {
        into_ref!(peri);

        Self { _peri: peri }
    }

    pub fn init(&mut self) {
        IPCC::enable();
        IPCC::reset();
        IPCC::set_cpu2(true);

        unsafe { _configure_pwr() };

        let regs = IPCC::regs();

        unsafe {
            regs.cpu(0).cr().modify(|w| {
                w.set_rxoie(true);
                w.set_txfie(true);
            })
        }
    }

    pub fn c1_set_rx_channel(&mut self, channel: IpccChannel, enabled: bool) {
        let regs = IPCC::regs();

        // If bit is set to 1 then interrupt is disabled
        unsafe {
            regs.cpu(0)
                .mr()
                .modify(|w| w.set_chom(channel.into(), !enabled))
        }
    }

    pub fn c1_get_rx_channel(&self, channel: IpccChannel) -> bool {
        let regs = IPCC::regs();

        // If bit is set to 1 then interrupt is disabled
        unsafe { !regs.cpu(0).mr().read().chom(channel.into()) }
    }

    pub fn c2_set_rx_channel(&mut self, channel: IpccChannel, enabled: bool) {
        let regs = IPCC::regs();

        // If bit is set to 1 then interrupt is disabled
        unsafe {
            regs.cpu(1)
                .mr()
                .modify(|w| w.set_chom(channel.into(), !enabled))
        }
    }

    pub fn c2_get_rx_channel(&self, channel: IpccChannel) -> bool {
        let regs = IPCC::regs();

        // If bit is set to 1 then interrupt is disabled
        unsafe { !regs.cpu(1).mr().read().chom(channel.into()) }
    }

    pub fn c1_set_tx_channel(&mut self, channel: IpccChannel, enabled: bool) {
        let regs = IPCC::regs();

        // If bit is set to 1 then interrupt is disabled
        unsafe {
            regs.cpu(0)
                .mr()
                .modify(|w| w.set_chfm(channel.into(), !enabled))
        }
    }

    pub fn c1_get_tx_channel(&self, channel: IpccChannel) -> bool {
        let regs = IPCC::regs();

        // If bit is set to 1 then interrupt is disabled
        unsafe { !regs.cpu(0).mr().read().chfm(channel.into()) }
    }

    pub fn c2_set_tx_channel(&mut self, channel: IpccChannel, enabled: bool) {
        let regs = IPCC::regs();

        // If bit is set to 1 then interrupt is disabled
        unsafe {
            regs.cpu(1)
                .mr()
                .modify(|w| w.set_chfm(channel.into(), !enabled))
        }
    }

    pub fn c2_get_tx_channel(&self, channel: IpccChannel) -> bool {
        let regs = IPCC::regs();

        // If bit is set to 1 then interrupt is disabled
        unsafe { !regs.cpu(1).mr().read().chfm(channel.into()) }
    }

    /// clears IPCC receive channel status for CPU1
    pub fn c1_clear_flag_channel(&mut self, channel: IpccChannel) {
        let regs = IPCC::regs();

        unsafe { regs.cpu(0).scr().write(|w| w.set_chc(channel.into(), true)) }
    }

    /// clears IPCC receive channel status for CPU2
    pub fn c2_clear_flag_channel(&mut self, channel: IpccChannel) {
        let regs = IPCC::regs();

        unsafe { regs.cpu(1).scr().write(|w| w.set_chc(channel.into(), true)) }
    }

    pub fn c1_set_flag_channel(&mut self, channel: IpccChannel) {
        let regs = IPCC::regs();

        unsafe { regs.cpu(0).scr().write(|w| w.set_chs(channel.into(), true)) }
    }

    pub fn c2_set_flag_channel(&mut self, channel: IpccChannel) {
        let regs = IPCC::regs();

        unsafe { regs.cpu(1).scr().write(|w| w.set_chs(channel.into(), true)) }
    }

    pub fn c1_is_active_flag(&self, channel: IpccChannel) -> bool {
        let regs = IPCC::regs();

        unsafe { regs.cpu(0).sr().read().chf(channel.into()) }
    }

    pub fn c2_is_active_flag(&self, channel: IpccChannel) -> bool {
        let regs = IPCC::regs();

        unsafe { regs.cpu(1).sr().read().chf(channel.into()) }
    }

    pub fn is_tx_pending(&self, channel: IpccChannel) -> bool {
        !self.c1_is_active_flag(channel) && self.c1_get_tx_channel(channel)
    }

    pub fn is_rx_pending(&self, channel: IpccChannel) -> bool {
        self.c2_is_active_flag(channel) && self.c1_get_rx_channel(channel)
    }
}

impl sealed::Instance for embassy_stm32::peripherals::IPCC {
    fn regs() -> embassy_stm32::pac::ipcc::Ipcc {
        embassy_stm32::pac::IPCC
    }

    fn set_cpu2(enabled: bool) {
        unsafe {
            embassy_stm32::pac::PWR
                .cr4()
                .modify(|w| w.set_c2boot(enabled))
        }
    }

    fn state() -> &'static State {
        static STATE: State = State::new();
        &STATE
    }
}

/// extension trait that constrains the [`Ipcc`] peripheral
pub trait IpccExt<'d> {
    fn constrain(self) -> Ipcc<'d>;
}
impl<'d> IpccExt<'d> for IPCC {
    fn constrain(self) -> Ipcc<'d> {
        Ipcc {
            _peri: self.into_ref(),
        }
    }
}

/// Fastest clock configuration.
/// * External low-speed crystal is used (LSE)
/// * 32 MHz HSE with PLL
/// * 64 MHz CPU1, 32 MHz CPU2
/// * 64 MHz for APB1, APB2
/// * HSI as a clock source after wake-up from low-power mode
unsafe fn _configure_pwr() {
    let _pwr = embassy_stm32::pac::PWR;
    let rcc = embassy_stm32::pac::RCC;

    rcc.cfgr().modify(|w| w.set_stopwuck(true));

    crate::pwr::set_backup_access(true);

    // configure LSE
    rcc.bdcr().modify(|w| w.set_lseon(true));

    // select system clock source = PLL
    // set PLL coefficients
    // m: 2,
    // n: 12,
    // r: 3,
    // q: 4,
    // p: 3,
    let src_bits = 0b11;
    let pllp = (3 - 1) & 0b11111;
    let pllq = (4 - 1) & 0b111;
    let pllr = (3 - 1) & 0b111;
    let plln = 12 & 0b1111111;
    let pllm = (2 - 1) & 0b111;
    rcc.pllcfgr().modify(|w| {
        w.set_pllsrc(src_bits);
        w.set_pllm(pllm);
        w.set_plln(plln);
        w.set_pllr(pllr);
        w.set_pllp(pllp);
        w.set_pllpen(true);
        w.set_pllq(pllq);
        w.set_pllqen(true);
    });
    // enable PLL
    rcc.cr().modify(|w| w.set_pllon(true));
    rcc.cr().write(|w| w.set_hsion(false));
    // while !rcc.cr().read().pllrdy() {}

    // configure SYSCLK mux to use PLL clocl
    rcc.cfgr().modify(|w| w.set_sw(0b11));

    // configure CPU1 & CPU2 dividers
    rcc.cfgr().modify(|w| w.set_hpre(0)); // not divided
    rcc.extcfgr().modify(|w| {
        w.set_c2hpre(0b1000); // div2
        w.set_shdhpre(0); // not divided
    });

    // apply APB1 / APB2 values
    rcc.cfgr().modify(|w| {
        w.set_ppre1(0b000); // not divided
        w.set_ppre2(0b000); // not divided
    });

    // TODO: required
    // set RF wake-up clock = LSE
    rcc.csr().modify(|w| w.set_rfwkpsel(0b01));

    // set LPTIM1 & LPTIM2 clock source
    rcc.ccipr().modify(|w| {
        w.set_lptim1sel(0b00); // PCLK
        w.set_lptim2sel(0b00); // PCLK
    });
}
