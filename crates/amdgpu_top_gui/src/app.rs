use std::sync::{Arc, Mutex};
use std::ops::RangeInclusive;
use std::path::PathBuf;
use eframe::egui;
use egui::{RichText, util::History};
use egui_plot::{Corner, Legend, Line, Plot, PlotPoint, PlotPoints};
use crate::{BASE, MEDIUM, HISTORY_LENGTH};

use libamdgpu_top::AMDGPU::{
    MetricsInfo,
    GPU_INFO,
    IpDieEntry,
};
use libamdgpu_top::stat::{self, gpu_metrics_util::*, FdInfoSortType, PerfCounter};

use crate::{AppDeviceInfo, CentralData, GpuMetrics, util::*, fl};

const PLOT_HEIGHT: f32 = 32.0;
const PLOT_WIDTH: f32 = 240.0;

pub struct MyApp {
    pub command_path: PathBuf,
    pub app_device_info: AppDeviceInfo,
    pub device_list: Vec<DeviceListMenu>,
    pub has_vcn_unified: bool,
    pub support_pcie_bw: bool,
    pub fdinfo_sort: FdInfoSortType,
    pub reverse_sort: bool,
    pub buf_data: CentralData,
    pub arc_data: Arc<Mutex<CentralData>>,
    pub show_sidepanel: bool,
    pub gl_vendor_info: Option<String>,
}

fn grid(ui: &mut egui::Ui, v: &[(&str, &str)]) {
    for (name, val) in v {
        ui.label(*name);
        ui.label(*val);
        ui.end_row();
    }
}

trait GuiInfo {
    fn device_info(&self, ui: &mut egui::Ui, gl_vendor_info: &Option<String>);
    fn gfx_info(&self, ui: &mut egui::Ui);
    fn memory_info(&self, ui: &mut egui::Ui);
    fn cache_info(&self, ui: &mut egui::Ui);
    fn power_cap_info(&self, ui: &mut egui::Ui);
    fn temp_info(&self, ui: &mut egui::Ui);
    fn fan_info(&self, ui: &mut egui::Ui);
    fn link_info(&self, ui: &mut egui::Ui);
}

impl GuiInfo for AppDeviceInfo {
    fn device_info(&self, ui: &mut egui::Ui, gl_vendor_info: &Option<String>) {
        let dev_id = format!("{:#0X}.{:#0X}", self.ext_info.device_id(), self.ext_info.pci_rev_id());

        grid(ui, &[
            (&fl!("device_name"), &self.marketing_name),
            (&fl!("pci_bus"), &self.pci_bus.to_string()),
            (&fl!("did_rid"), &dev_id),
        ]);

        if let Some(gl) = gl_vendor_info {
            ui.label(&fl!("opengl_driver_ver"));
            ui.label(gl);
            ui.end_row();
        }

        ui.end_row();
    }

    fn gfx_info(&self, ui: &mut egui::Ui) {
        let gpu_type = if self.ext_info.is_apu() { fl!("apu") } else { fl!("dgpu") };
        let family = self.ext_info.get_family_name();
        let asic = self.ext_info.get_asic_name();
        let chip_class = self.ext_info.get_chip_class();
        let max_good_cu_per_sa = self.ext_info.get_max_good_cu_per_sa();
        let min_good_cu_per_sa = self.ext_info.get_min_good_cu_per_sa();
        let cu_per_sa = if max_good_cu_per_sa != min_good_cu_per_sa {
            format!("[{min_good_cu_per_sa}, {max_good_cu_per_sa}]")
        } else {
            max_good_cu_per_sa.to_string()
        };
        let rb_pipes = self.ext_info.rb_pipes();
        let rop_count = self.ext_info.calc_rop_count();
        let rb_type = if asic.rbplus_allowed() {
            fl!("rb")
        } else {
            fl!("rb_plus")
        };
        let peak_gp = format!("{} {}", rop_count * self.max_gpu_clk / 1000, fl!("gp_s"));
        let peak_fp32 = format!("{} {}", self.ext_info.peak_gflops(), fl!("gflops"));

        grid(ui, &[
            (&fl!("gpu_type"), &gpu_type),
            (&fl!("family"), &family.to_string()),
            (&fl!("asic_name"), &asic.to_string()),
            (&fl!("chip_class"), &chip_class.to_string()),
            (&fl!("shader_engine"), &self.ext_info.max_se().to_string()),
            (&fl!("shader_array_per_se"), &self.ext_info.max_sa_per_se().to_string()),
            (&fl!("cu_per_sa"), &cu_per_sa),
            (&fl!("total_cu"), &self.ext_info.cu_active_number().to_string()),
            (&rb_type, &format!("{rb_pipes} ({rop_count} ROPs)")),
            (&fl!("peak_gp"), &peak_gp),
            (&fl!("gpu_clock"), &format!("{}-{} MHz", self.min_gpu_clk, self.max_gpu_clk)),
            (&fl!("peak_fp32"), &peak_fp32),
        ]);
        ui.end_row();
    }

    fn memory_info(&self, ui: &mut egui::Ui) {
        let re_bar = if self.resizable_bar {
            fl!("enabled")
        } else {
            fl!("disabled")
        };

        grid(ui, &[
            (&fl!("vram_type"), &self.ext_info.get_vram_type().to_string()),
            (&fl!("vram_bit_width"), &format!("{}-{}", self.ext_info.vram_bit_width, fl!("bit"))),
            (&fl!("vram_size"), &format!("{} {}", self.memory_info.vram.total_heap_size >> 20, fl!("mib"))),
            (&fl!("memory_clock"), &format!("{}-{} {}", self.min_mem_clk, self.max_mem_clk, fl!("mhz"))),
            (&fl!("resizable_bar"), &re_bar),
        ]);
        ui.end_row();
    }

    fn cache_info(&self, ui: &mut egui::Ui) {
        let kib = fl!("kib");
        let mib = fl!("mib");
        let banks = fl!("banks");

        ui.label(fl!("l1_cache_per_cu"));
        ui.label(format!("{:4} {kib}", self.l1_cache_size_kib_per_cu));
        ui.end_row();
        if 0 < self.gl1_cache_size_kib_per_sa {
            ui.label(fl!("gl1_cache_per_sa"));
            ui.label(format!("{:4} {kib}", self.gl1_cache_size_kib_per_sa));
            ui.end_row();
        }
        ui.label(fl!("l2_cache"));
        ui.label(format!(
            "{:4} {kib} ({} {banks})",
            self.total_l2_cache_size_kib,
            self.actual_num_tcc_blocks,
        ));
        ui.end_row();
        if 0 < self.total_l3_cache_size_mib {
            ui.label(fl!("l3_cache"));
            ui.label(format!(
                "{:4} {mib} ({} {banks})",
                self.total_l3_cache_size_mib,
                self.actual_num_tcc_blocks,
            ));
            ui.end_row();
        }
        ui.end_row();
    }

    fn power_cap_info(&self, ui: &mut egui::Ui) {
        let Some(cap) = &self.power_cap else { return };

        ui.label(fl!("power_cap"));
        ui.label(format!("{:4} W ({}-{} W)", cap.current, cap.min, cap.max));
        ui.end_row();
        ui.label(fl!("power_cap_default"));
        ui.label(format!("{:4} W", cap.default));
        ui.end_row();
    }

    fn temp_info(&self, ui: &mut egui::Ui) {
        for temp in [
            &self.edge_temp,
            &self.junction_temp,
            &self.memory_temp,
        ] {
            let Some(temp) = temp else { continue };
            let name = temp.type_.to_string();
            if let Some(crit) = temp.critical {
                ui.label(format!("{name} Temp. (Critical)"));
                ui.label(format!("{crit:4} C"));
                ui.end_row();
            }
            if let Some(e) = temp.emergency {
                ui.label(format!("{name} Temp. (Emergency)"));
                ui.label(format!("{e:4} C"));
                ui.end_row();
            }
        }
    }

    fn fan_info(&self, ui: &mut egui::Ui) {
        let Some(fan_rpm) = &self.fan_max_rpm else { return };

        ui.label("Fan RPM (Max)");
        ui.label(format!("{fan_rpm:4} RPM"));
        ui.end_row();
    }

    fn link_info(&self, ui: &mut egui::Ui) {
        let pcie_link_speed = fl!("pcie_link_speed");
        let fl_max = fl!("max");
        let dpm = fl!("dpm");
        if let [Some(min), Some(max)] = [&self.min_dpm_link, &self.max_dpm_link] {
            ui.label(format!("{pcie_link_speed} ({dpm})"));
            ui.label(format!("Gen{}x{} - Gen{}x{}", min.gen, min.width, max.gen, max.width));
            ui.end_row();
        } else if let Some(max) = &self.max_dpm_link {
            ui.label(format!("{pcie_link_speed} ({dpm}, {fl_max})"));
            ui.label(format!("Gen{}x{}", max.gen, max.width));
            ui.end_row();
        }

        if let Some(gpu) = &self.max_gpu_link {
            ui.label(format!("{pcie_link_speed} ({}, {fl_max})", fl!("gpu")));
            ui.label(format!("Gen{}x{}", gpu.gen, gpu.width));
            ui.end_row();
        }

        if let Some(system) = &self.max_system_link {
            ui.label(format!("{pcie_link_speed} ({}, {fl_max})", fl!("system")));
            ui.label(format!("Gen{}x{}", system.gen, system.width));
            ui.end_row();
        }
    }
}

impl MyApp {
    pub fn egui_app_device_info(&self, ui: &mut egui::Ui, gl_vendor_info: &Option<String>) {
        egui::Grid::new("app_device_info").show(ui, |ui| {
            self.app_device_info.device_info(ui, gl_vendor_info);
            self.app_device_info.gfx_info(ui);
            self.app_device_info.memory_info(ui);
            self.app_device_info.cache_info(ui);
            self.app_device_info.power_cap_info(ui);
            self.app_device_info.temp_info(ui);
            self.app_device_info.fan_info(ui);
            self.app_device_info.link_info(ui);

            let profiles: Vec<String> = self.app_device_info.power_profiles.iter().map(|p| p.to_string()).collect();

            ui.label(fl!("supported_power_profiles").to_string());
            ui.label(format!("{profiles:#?}"));
            ui.end_row();
        });
    }

    pub fn egui_ip_discovery_table(&self, ui: &mut egui::Ui) {
        let gpu_die = fl!("gpu_die");
        for die in &self.app_device_info.ip_die_entries {
            let label = format!("{gpu_die}: {}", die.die_id);
            collapsing(ui, &label, false, |ui| Self::egui_ip_discovery_table_per_die(die, ui));
        }
    }

    pub fn egui_ip_discovery_table_per_die(ip_die_entry: &IpDieEntry, ui: &mut egui::Ui) {
        egui::Grid::new(format!("ip_discovery_table die{}", ip_die_entry.die_id)).show(ui, |ui| {
            ui.label(fl!("ip_hw")).highlight();
            ui.label(fl!("version")).highlight();
            ui.label(fl!("num")).highlight();
            ui.end_row();

            for ip_hw in &ip_die_entry.ip_hw_ids {
                let hw_id = ip_hw.hw_id.to_string();
                let Some(inst) = ip_hw.instances.first() else { continue };
                ui.label(hw_id);
                ui.label(format!("{}.{}.{}", inst.major, inst.minor, inst.revision));
                ui.label(ip_hw.instances.len().to_string());
                ui.end_row();
            }
        });
    }

    pub fn egui_video_caps_info(&self, ui: &mut egui::Ui) {
        let Some(decode_caps) = &self.app_device_info.decode else { return };
        let Some(encode_caps) = &self.app_device_info.encode else { return };

        egui::Grid::new("codec_info").show(ui, |ui| {
            ui.label(fl!("codec")).highlight();
            ui.label(fl!("decode")).highlight();
            ui.label(fl!("encode")).highlight();
            ui.end_row();

            let n_a = fl!("n_a");
            
            for (name, decode, encode) in [
                ("MPEG2", decode_caps.mpeg2, encode_caps.mpeg2),
                ("MPEG4", decode_caps.mpeg4, encode_caps.mpeg4),
                ("VC1", decode_caps.vc1, encode_caps.vc1),
                ("MPEG4_AVC", decode_caps.mpeg4_avc, encode_caps.mpeg4_avc),
                ("HEVC", decode_caps.hevc, encode_caps.hevc),
                ("JPEG", decode_caps.jpeg, encode_caps.jpeg),
                ("VP9", decode_caps.vp9, encode_caps.vp9),
                ("AV1", decode_caps.av1, encode_caps.av1),
            ] {
                ui.label(name);
                if let Some(dec) = decode {
                    ui.label(&format!("{}x{}", dec.max_width, dec.max_height));
                } else {
                    ui.label(&n_a);
                }
                if let Some(enc) = encode {
                    ui.label(&format!("{}x{}", enc.max_width, enc.max_height));
                } else {
                    ui.label(&n_a);
                }
                ui.end_row();
            }
        });
    }

    pub fn egui_vbios_info(&self, ui: &mut egui::Ui) {
        let Some(vbios) = &self.app_device_info.vbios else { return };
        egui::Grid::new("vbios_info").show(ui, |ui| {
            for (name, val) in [
                (fl!("vbios_name"), &vbios.name),
                (fl!("vbios_pn"), &vbios.pn),
                (fl!("vbios_version"), &vbios.ver),
                (fl!("vbios_date"), &vbios.date),
            ] {
                ui.label(name).highlight();
                ui.label(val);
                ui.end_row();
            }
        });
    }

    pub fn egui_perf_counter(
        &self,
        ui: &mut egui::Ui,
        name: &str,
        pc: &PerfCounter,
        history: &[History<u8>],
    ) {
        let label_fmt = |_s: &str, val: &PlotPoint| {
            format!("{:.1}s : {:.0}%", val.x, val.y)
        };

        egui::Grid::new(name).show(ui, |ui| {
            for ((name, pos), history) in pc.index.iter().zip(history.iter()) {
                let usage = pc.bits.get(*pos);
                ui.label(name);
                ui.label(format!("{usage:3}%"));

                let points: PlotPoints = history.iter()
                    .map(|(i, val)| [i, val as f64]).collect();
                let line = Line::new(points).fill(1.0);
                Plot::new(name)
                    .allow_drag(false)
                    .allow_zoom(false)
                    .allow_scroll(false)
                    .include_y(0.0)
                    .include_y(100.0)
                    .y_axis_formatter(empty_y_fmt)
                    .label_formatter(label_fmt)
                    .auto_bounds_x()
                    .height(PLOT_HEIGHT)
                    .width(PLOT_WIDTH)
                    .show(ui, |plot_ui| plot_ui.line(line));
                ui.end_row();
            }
        });
    }

    pub fn egui_vram(&self, ui: &mut egui::Ui) {
        egui::Grid::new("VRAM").show(ui, |ui| {
            let mib = fl!("mib");
            for (v, name) in [
                (&self.buf_data.vram_usage.0.vram, fl!("vram")),
                (&self.buf_data.vram_usage.0.cpu_accessible_vram, fl!("cpu_visible_vram")),
                (&self.buf_data.vram_usage.0.gtt, fl!("gtt")),
            ] {
                let progress = (v.heap_usage >> 20) as f32 / (v.total_heap_size >> 20) as f32;
                let text = format!("{:5} / {:5} {mib}", v.heap_usage >> 20, v.total_heap_size >> 20);
                let bar = egui::ProgressBar::new(progress)
                    .text(RichText::new(&text).font(BASE));
                ui.label(RichText::new(name).font(MEDIUM));
                ui.add_sized([360.0, 16.0], bar);
                ui.end_row();
            }
        });
    }

    fn set_fdinfo_sort_type(&mut self, sort_type: FdInfoSortType) {
        if sort_type == self.fdinfo_sort {
            self.reverse_sort ^= true;
        } else {
            self.reverse_sort = false;
        }
        self.fdinfo_sort = sort_type;
    }

    pub fn egui_fdinfo_plot(&self, ui: &mut egui::Ui) {
        let label_fmt = |name: &str, val: &PlotPoint| {
            format!("{:.1}s : {name} {:.0}%", val.x, val.y)
        };

        let [mut gfx, mut compute, mut dma, mut dec, mut enc] = [0; 5]
            .map(|_| Vec::<[f64; 2]>::with_capacity(HISTORY_LENGTH.end));

        for (i, usage) in self.buf_data.fdinfo_history.iter() {
            let usage_dec = usage.dec + usage.vcn_jpeg;
            let usage_enc = usage.enc + usage.uvd_enc;

            gfx.push([i, usage.gfx as f64]);
            compute.push([i, usage.compute as f64]);
            dma.push([i, usage.dma as f64]);
            dec.push([i, usage_dec as f64]);
            enc.push([i, usage_enc as f64]);
        }

        Plot::new(fl!("fdinfo_plot"))
            .allow_drag(false)
            .allow_zoom(false)
            .allow_scroll(false)
            .include_y(0.0)
            .include_y(100.0)
            .y_axis_formatter(empty_y_fmt)
            .label_formatter(label_fmt)
            .auto_bounds_x()
            .height(ui.available_width() / 4.0)
            .width(ui.available_width() - 36.0)
            .legend(Legend::default().position(Corner::LeftTop))
            .show(ui, |plot_ui| {
                for (usage, name) in [
                    (gfx, fl!("gfx")),
                    (compute, fl!("compute")),
                    (dma, fl!("dma")),
                ] {
                    plot_ui.line(Line::new(PlotPoints::new(usage)).name(name));
                }

                if self.has_vcn_unified {
                    plot_ui.line(Line::new(PlotPoints::new(enc)).name(fl!("media")));
                } else {
                    plot_ui.line(Line::new(PlotPoints::new(dec)).name(fl!("decode")));
                    plot_ui.line(Line::new(PlotPoints::new(enc)).name(fl!("encode")));
                }
            });
    }

    pub fn egui_grid_fdinfo(&mut self, ui: &mut egui::Ui) {
        collapsing_plot(ui, "fdinfo Plot", true, |ui| self.egui_fdinfo_plot(ui));

        egui::Grid::new("fdinfo").show(ui, |ui| {
            ui.style_mut().override_font_id = Some(MEDIUM);
            ui.label(rt_base(format!("{:^15}", fl!("name")))).highlight();
            ui.label(rt_base(format!("{:^8}", fl!("pid")))).highlight();
            if ui.button(rt_base(format!("{:^10}", fl!("vram")))).clicked() {
                self.set_fdinfo_sort_type(FdInfoSortType::VRAM);
            }
            if ui.button(rt_base(format!("{:^10}", fl!("gtt")))).clicked() {
                self.set_fdinfo_sort_type(FdInfoSortType::GTT);
            }
            if ui.button(rt_base(format!("{:^5}", fl!("cpu")))).clicked() {
                self.set_fdinfo_sort_type(FdInfoSortType::CPU);
            }
            if ui.button(rt_base(format!("{:^5}", fl!("gfx")))).clicked() {
                self.set_fdinfo_sort_type(FdInfoSortType::GFX);
            }
            if ui.button(rt_base(fl!("compute"))).clicked() {
                self.set_fdinfo_sort_type(FdInfoSortType::Compute);
            }
            if ui.button(rt_base(format!("{:^5}", fl!("dma")))).clicked() {
                self.set_fdinfo_sort_type(FdInfoSortType::DMA);
            }
            if self.has_vcn_unified {
                if ui.button(rt_base(format!("{:^5}", fl!("media")))).clicked() {
                    self.set_fdinfo_sort_type(FdInfoSortType::Encode);
                }
            } else {
                if ui.button(rt_base(fl!("decode"))).clicked() {
                    self.set_fdinfo_sort_type(FdInfoSortType::Decode);
                }
                if ui.button(rt_base(fl!("encode"))).clicked() {
                    self.set_fdinfo_sort_type(FdInfoSortType::Encode);
                }
            }
            ui.end_row();

            stat::sort_proc_usage(
                &mut self.buf_data.fdinfo.proc_usage,
                &self.fdinfo_sort,
                self.reverse_sort,
            );

            let mib = fl!("mib");

            for pu in &self.buf_data.fdinfo.proc_usage {
                ui.label(pu.name.to_string());
                ui.label(format!("{:>8}", pu.pid));
                ui.label(format!("{:5} {mib}", pu.usage.vram_usage >> 10));
                ui.label(format!("{:5} {mib}", pu.usage.gtt_usage >> 10));
                for usage in [
                    pu.cpu_usage,
                    pu.usage.gfx,
                    pu.usage.compute,
                    pu.usage.dma,
                ] {
                    ui.label(format!("{usage:3} %"));
                }

                if self.has_vcn_unified {
                    ui.label(format!("{:3} %", pu.usage.media));
                } else {
                    let dec_usage = pu.usage.dec + pu.usage.vcn_jpeg;
                    let enc_usage = pu.usage.enc + pu.usage.uvd_enc;
                    ui.label(format!("{dec_usage:3} %"));
                    ui.label(format!("{enc_usage:3} %"));
                }
                ui.end_row();
            } // proc_usage
        });
    }

    pub fn egui_sensors(&self, ui: &mut egui::Ui) {
        ui.style_mut().override_font_id = Some(MEDIUM);
        let sensors = &self.buf_data.sensors;
        egui::Grid::new("Sensors").show(ui, |ui| {
            for (history, val, label, min, max, unit) in [
                (
                    &self.buf_data.sensors_history.sclk,
                    sensors.sclk,
                    "GFX_SCLK",
                    self.app_device_info.min_gpu_clk,
                    self.app_device_info.max_gpu_clk,
                    fl!("mhz"),
                ),
                (
                    &self.buf_data.sensors_history.mclk,
                    sensors.mclk,
                    "GFX_MCLK",
                    self.app_device_info.min_mem_clk,
                    self.app_device_info.max_mem_clk,
                    fl!("mhz"),
                ),
                (
                    &self.buf_data.sensors_history.vddgfx,
                    sensors.vddgfx,
                    "VDDGFX",
                    500, // "500 mV" is not an exact value
                    1500, // "1500 mV" is not an exact value
                    fl!("mv"),
                ),
                (
                    &self.buf_data.sensors_history.power,
                    sensors.power,
                    "GFX Power",
                    0,
                    if let Some(ref cap) = sensors.power_cap { cap.current } else { 350 }, // "350 W" is not an exact value
                    fl!("w"),
                ),
                (
                    &self.buf_data.sensors_history.fan_rpm,
                    sensors.fan_rpm,
                    "Fan",
                    0,
                    sensors.fan_max_rpm.unwrap_or(6000), // "6000 RPM" is not an exact value
                    fl!("rpm"),
                ),
            ] {
                let Some(val) = val else { continue };

                ui.label(format!("{label}\n({val:4} {unit})"));

                if min == max {
                    ui.end_row();
                    continue;
                }

                let label_fmt = move |_name: &str, val: &PlotPoint| {
                    format!("{:.1}s\n{:.0} {unit}", val.x, val.y)
                };
                let points: PlotPoints = history.iter()
                    .map(|(i, val)| [i, val as f64]).collect();
                let line = Line::new(points).fill(1.0);
                Plot::new(label)
                    .allow_zoom(false)
                    .allow_scroll(false)
                    .include_y(min)
                    .include_y(max)
                    .y_axis_formatter(empty_y_fmt)
                    .label_formatter(label_fmt)
                    .auto_bounds_x()
                    .height(PLOT_HEIGHT * 1.5)
                    .width(PLOT_WIDTH)
                    .show(ui, |plot_ui| plot_ui.line(line));
                ui.end_row();
            }
        });

        self.egui_temp_plot(ui);

        if let Some(cur) = sensors.current_link {
            let min_max = if let [Some(min), Some(max)] = [sensors.min_dpm_link, sensors.max_dpm_link] {
                format!(
                    " (Gen{}x{} - Gen{}x{})",
                    min.gen,
                    min.width,
                    max.gen,
                    max.width,
                )
            } else if let Some(max) = sensors.max_dpm_link {
                format!(" ({} Gen{}x{})", fl!("max"), max.gen, max.width)
            } else {
                String::new()
            };

            ui.label(format!(
                "{} => Gen{}x{} {min_max}",
                fl!("pcie_link_speed"),
                cur.gen,
                cur.width,
            ));
        }
    }

    pub fn egui_temp_plot(&self, ui: &mut egui::Ui) {
        ui.style_mut().override_font_id = Some(MEDIUM);
        let sensors = &self.buf_data.sensors;
        let label_fmt = |_name: &str, val: &PlotPoint| {
            format!("{:.1}s\n{:.0} C", val.x, val.y)
        };

        egui::Grid::new("Temp. Sensors").show(ui, |ui| {
            for (label, temp, temp_history) in [
                ("Edge", &sensors.edge_temp, &self.buf_data.sensors_history.edge_temp),
                ("Junction", &sensors.junction_temp, &self.buf_data.sensors_history.junction_temp),
                ("Memory", &sensors.memory_temp, &self.buf_data.sensors_history.memory_temp),
            ] {
                let Some(temp) = temp else { continue };
                let val = temp.current;
                let max = temp.critical.unwrap_or(105) as f64;

                ui.label(format!("{label} Temp.\n({val:4} C)"));

                let points: PlotPoints = temp_history.iter()
                    .map(|(i, val)| [i, val as f64]).collect();
                let line = Line::new(points).fill(1.0);
                Plot::new(label)
                    .allow_zoom(false)
                    .allow_scroll(false)
                    .include_y(0.0)
                    .include_y(max)
                    .y_axis_formatter(empty_y_fmt)
                    .label_formatter(label_fmt)
                    .auto_bounds_x()
                    .auto_bounds_y()
                    .height(PLOT_HEIGHT * 1.5)
                    .width(PLOT_WIDTH)
                    .show(ui, |plot_ui| plot_ui.line(line));
                ui.end_row();
            }
        });
    }

    pub fn egui_pcie_bw(&self, ui: &mut egui::Ui) {
        let label_fmt = |name: &str, val: &PlotPoint| {
            format!("{:.1}s : {name} {:.0} {}", val.x, val.y, fl!("mib_s"))
        };

        let fl_sent = fl!("sent");
        let fl_rec = fl!("received");
        let mib_s = fl!("mib_s");

        let [sent, rec] = {
            let [mut sent_history, mut rec_history] = [0; 2].map(|_| Vec::<[f64; 2]>::new());

            for (i, (sent, rec)) in self.buf_data.pcie_bw_history.iter() {
                sent_history.push([i, sent as f64]);
                rec_history.push([i, rec as f64]);
            }

            [
                Line::new(PlotPoints::new(sent_history)).name(&fl_sent),
                Line::new(PlotPoints::new(rec_history)).name(&fl_rec),
            ]
        };

        Plot::new("pcie_bw plot")
            .allow_zoom(false)
            .allow_scroll(false)
            .include_y(0.0)
            .y_axis_formatter(empty_y_fmt)
            .label_formatter(label_fmt)
            .auto_bounds_x()
            .auto_bounds_y()
            .height(ui.available_width() / 4.0)
            .width(ui.available_width() - 36.0)
            .legend(Legend::default().position(Corner::LeftTop))
            .show(ui, |plot_ui| {
                plot_ui.line(sent);
                plot_ui.line(rec);
            });

        if let Some((sent, rec)) = self.buf_data.pcie_bw_history.latest() {
            ui.label(format!("{fl_sent}: {sent:5} {mib_s}, {fl_rec}: {rec:5} {mib_s}"));
        } else {
            ui.label(format!("{fl_sent}: _ {mib_s}, {fl_rec}: _ {mib_s}"));
        }
    }

    pub fn egui_gpu_metrics_v1(&self, ui: &mut egui::Ui) {
        let gpu_metrics = &self.buf_data.gpu_metrics;

        socket_power(ui, gpu_metrics);
        avg_activity(ui, gpu_metrics);

        ui.horizontal(|ui| {
            v1_helper(ui, &fl!("c"), &[
                (gpu_metrics.get_temperature_vrgfx(), "VRGFX"),
                (gpu_metrics.get_temperature_vrsoc(), "VRSOC"),
                (gpu_metrics.get_temperature_vrmem(), "VRMEM"),
            ]);
        });

        ui.horizontal(|ui| {
            v1_helper(ui, &fl!("mv"), &[
                (gpu_metrics.get_voltage_soc(), "SoC"),
                (gpu_metrics.get_voltage_gfx(), "GFX"),
                (gpu_metrics.get_voltage_mem(), "Mem"),
            ]);
        });

        let fl_avg = fl!("avg");
        let fl_cur = fl!("cur");
        let mhz = fl!("mhz");

        for (avg, cur, name) in [
            (
                gpu_metrics.get_average_gfxclk_frequency(),
                gpu_metrics.get_current_gfxclk(),
                "GFXCLK",
            ),
            (
                gpu_metrics.get_average_socclk_frequency(),
                gpu_metrics.get_current_socclk(),
                "SOCCLK",
            ),
            (
                gpu_metrics.get_average_uclk_frequency(),
                gpu_metrics.get_current_uclk(),
                "UMCCLK",
            ),
            (
                gpu_metrics.get_average_vclk_frequency(),
                gpu_metrics.get_current_vclk(),
                "VCLK",
            ),
            (
                gpu_metrics.get_average_dclk_frequency(),
                gpu_metrics.get_current_dclk(),
                "DCLK",
            ),
            (
                gpu_metrics.get_average_vclk1_frequency(),
                gpu_metrics.get_current_vclk1(),
                "VCLK1",
            ),
            (
                gpu_metrics.get_average_dclk1_frequency(),
                gpu_metrics.get_current_dclk1(),
                "DCLK1",
            ),
        ] {
            let [avg, cur] = [avg, cur].map(check_metrics_val);
            ui.label(format!("{name:<6} => {fl_avg} {avg:>4} {mhz}, {fl_cur} {cur:>4} {mhz}"));
        }

        // Only Aldebaran (MI200) supports it.
        if let Some(hbm_temp) = check_hbm_temp(gpu_metrics.get_temperature_hbm()) {
            ui.horizontal(|ui| {
                ui.label(format!("{} => [", fl!("hbm_temp")));
                for v in &hbm_temp {
                    ui.label(RichText::new(format!("{v:>5},")));
                }
                ui.label("]");
            });
        }

        throttle_status(ui, gpu_metrics);
    }

    pub fn egui_gpu_metrics_v2(&self, ui: &mut egui::Ui) {
        let gpu_metrics = &self.buf_data.gpu_metrics;
        let mhz = fl!("mhz");
        let mw = fl!("mw");

        ui.horizontal(|ui| {
            ui.label(format!("{} =>", fl!("gfx")));
            let temp_gfx = gpu_metrics.get_temperature_gfx().map(|v| v.saturating_div(100));
            v2_helper(ui, &[
                (temp_gfx, "C"),
                (gpu_metrics.get_average_gfx_power(), &mw),
                (gpu_metrics.get_current_gfxclk(), &mhz),
            ]);
        });

        ui.horizontal(|ui| {
            ui.label(format!("{} =>", fl!("soc")));
            let temp_soc = gpu_metrics.get_temperature_soc().map(|v| v.saturating_div(100));
            v2_helper(ui, &[
                (temp_soc, "C"),
                (gpu_metrics.get_average_soc_power(), &mw),
                (gpu_metrics.get_current_socclk(), &mhz),
            ]);
        });

        /*
            Most APUs return `average_socket_power` in mW,
            but Renoir APU (Renoir, Lucienne, Cezanne, Barcelo) return in W
            depending on the power management firmware version.  

            ref: drivers/gpu/drm/amd/pm/swsmu/smu12/renoir_ppt.c
            ref: https://gitlab.freedesktop.org/drm/amd/-/issues/2321
        */
        // socket_power(ui, gpu_metrics);
        avg_activity(ui, gpu_metrics);

        let fl_avg = fl!("avg");
        let fl_cur = fl!("cur");

        for (avg, cur, name) in [
            (
                gpu_metrics.get_average_uclk_frequency(),
                gpu_metrics.get_current_uclk(),
                "UMCCLK",
            ),
            (
                gpu_metrics.get_average_fclk_frequency(),
                gpu_metrics.get_current_fclk(),
                "FCLK",
            ),
            (
                gpu_metrics.get_average_vclk_frequency(),
                gpu_metrics.get_current_vclk(),
                "VCLK",
            ),
            (
                gpu_metrics.get_average_dclk_frequency(),
                gpu_metrics.get_current_dclk(),
                "DCLK",
            ),
        ] {
            let [avg, cur] = [avg, cur].map(check_metrics_val);
            ui.label(format!("{name:<6} => {fl_avg} {avg:>4} {mhz}, {fl_cur} {cur:>4} {mhz}"));
        }

        egui::Grid::new("GPU Metrics v2.x Core/L3").show(ui, |ui| {
            let core_temp = check_temp_array(gpu_metrics.get_temperature_core());
            let l3_temp = check_temp_array(gpu_metrics.get_temperature_l3());
            let [core_power, core_clk] = [
                gpu_metrics.get_average_core_power(),
                gpu_metrics.get_current_coreclk(),
            ].map(check_power_clock_array);
            let l3_clk = check_power_clock_array(gpu_metrics.get_current_l3clk());

            for (val, label) in [
                (core_temp, fl!("core_temp")),
                (core_power, fl!("core_power")),
                (core_clk, fl!("core_clock")),
            ] {
                let Some(val) = val else { continue };
                ui.label(label);
                ui.label("=> [");
                for v in &val {
                    ui.label(RichText::new(format!("{v:>5},")));
                }
                ui.label("]");
                ui.end_row();
            }

            for (val, label) in [
                (l3_temp, fl!("l3_temp")),
                (l3_clk, fl!("l3_clock")),
            ] {
                let Some(val) = val else { continue };
                ui.label(label);
                ui.label("=> [");
                for v in &val {
                    ui.label(RichText::new(format!("{v:>5},")));
                }
                ui.label("]");
                ui.end_row();
            }

            for (label, voltage, current) in [
                (
                    fl!("cpu"),
                    gpu_metrics.get_average_cpu_voltage(),
                    gpu_metrics.get_average_cpu_current(),
                ),
                (
                    fl!("soc"),
                    gpu_metrics.get_average_soc_voltage(),
                    gpu_metrics.get_average_soc_current(),
                ),
                (
                    fl!("gfx"),
                    gpu_metrics.get_average_gfx_voltage(),
                    gpu_metrics.get_average_gfx_current(),
                ),
            ] {
                let Some(voltage) = voltage else { continue };
                let Some(current) = current else { continue };

                ui.label(format!(
                    "{label} => {voltage:>5} {mv}, {current:>5} {ma}",
                    mv = fl!("mv"),
                    ma = fl!("ma"),
                ));
            }
        });

        throttle_status(ui, gpu_metrics);
    }
}

fn empty_y_fmt(_y: f64, _max_len: usize, _range: &RangeInclusive<f64>) -> String {
    String::new()
}

fn socket_power(ui: &mut egui::Ui, gpu_metrics: &GpuMetrics) {
    let v = check_metrics_val(gpu_metrics.get_average_socket_power());
    ui.label(format!("{} => {v:>3} W", fl!("socket_power")));
}

fn avg_activity(ui: &mut egui::Ui, gpu_metrics: &GpuMetrics) {
    ui.horizontal(|ui| {
        ui.label(format!("{} =>", fl!("avg_activity")));
        let activity = stat::GpuActivity::from_gpu_metrics(gpu_metrics);

        for (val, label) in [
            (activity.gfx, fl!("gfx")),
            (activity.umc, fl!("memory")),
            (activity.media, fl!("media")),
        ] {
            if let Some(val) = val {
                ui.label(format!("{label} {val:>3}%,"));
            } else {
                ui.label(format!("{label} ___%,"));
            }
        }
    });
}

fn throttle_status(ui: &mut egui::Ui, gpu_metrics: &GpuMetrics) {
    if let Some(thr) = gpu_metrics.get_throttle_status_info() {
        ui.label(
            format!(
                "{}: {:?}",
                fl!("throttle_status"),
                thr.get_all_throttler(),
            )
        );
    }
}

fn v1_helper(ui: &mut egui::Ui, unit: &str, v: &[(Option<u16>, &str)]) {
    for (val, name) in v {
        let v = check_metrics_val(*val);
        ui.label(format!("{name} => {v:>4} {unit}, "));
    }
}

fn v2_helper(ui: &mut egui::Ui, v: &[(Option<u16>, &str)]) {
    for (val, unit) in v {
        let v = check_metrics_val(*val);
        ui.label(format!("{v:>5} {unit}, "));
    }
}
