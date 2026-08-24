use std::fs::File;
use std::io::Read;
use std::path::Path;

use anyhow::{bail, Context, Result};

use crate::{
    apk,
    compat::{self, CompatibilityReport},
    dalvik::{DexFile, DexHeader, ExecutionResult},
    framework::Framework,
    framework::{ActivityManager, Intent, Value},
    manifest::AppManifest,
    resources::ResourceTable,
    vm::{ObjectId, Value as VmValue, Vm, VmConfig},
    HostGles, VirtualScreen,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeConfig {
    pub screen: VirtualScreen,
    pub api_level: u32,
    pub release: &'static str,
    pub max_steps: usize,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            screen: VirtualScreen::default(),
            api_level: crate::ANDROID_X_MAX_API_LEVEL,
            release: "Android 1.x",
            max_steps: 100_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchReport {
    pub package: String,
    pub dex: String,
    pub launcher_activity: String,
    pub application_label: Option<String>,
    pub message: String,
    pub compatibility: CompatibilityReport,
}

pub struct Runtime {
    pub config: RuntimeConfig,
    pub activities: ActivityManager,
    pub graphics: HostGles,
    pub framework: Framework,
    pub session: Option<RuntimeSession>,
    pub game_title: Option<String>,
}

pub struct RuntimeSession {
    pub vm: Vm<'static>,
    pub listener: ObjectId,
    pub render_method: String,
}

impl RuntimeSession {
    pub fn render_frame(&mut self, _width: u32, _height: u32) -> Result<(usize, usize)> {
        let logical_width = self.vm.framework.gles.framebuffer().width().max(1);
        let logical_height = self.vm.framework.gles.framebuffer().height().max(1);
        self.vm.framework.surface_size = (logical_width as i32, logical_height as i32);
        self.vm.framework.gles.begin_frame();
        self.vm
            .framework
            .gles
            .viewport(0, 0, logical_width, logical_height);
        self.vm
            .render_frame(self.listener, &self.render_method)
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        crate::publish_framebuffer(self.vm.framework.gles.framebuffer());
        Ok((
            self.vm.framework.gles.command_count(),
            self.vm.framework.gles.rendered_pixels(),
        ))
    }

    pub fn render_current_frame(&mut self) -> Result<(usize, usize)> {
        self.render_frame(0, 0)
    }
}

impl Default for Runtime {
    fn default() -> Self {
        let config = RuntimeConfig::default();
        Self {
            graphics: HostGles::new(config.screen),
            config,
            activities: ActivityManager::default(),
            framework: Framework::new(),
            session: None,
            game_title: None,
        }
    }
}

impl Runtime {
    pub fn validate_apk(&self, path: impl AsRef<Path>) -> Result<LaunchReport> {
        let path = path.as_ref();
        let info = apk::inspect(path)?;
        if !info.has_manifest {
            bail!("APK has no AndroidManifest.xml");
        }
        if !info.has_dex {
            bail!("APK has no classes.dex");
        }
        let file = File::open(path).with_context(|| format!("open APK {}", path.display()))?;
        let mut archive = zip::ZipArchive::new(file)?;
        let manifest_bytes = read_entry(&mut archive, "AndroidManifest.xml")?;
        let dex_bytes = read_entry(&mut archive, "classes.dex")?;
        let manifest = AppManifest::parse_axml(&manifest_bytes)?;
        let dex = DexFile::parse(&dex_bytes)?;
        let resources = if info.entries.iter().any(|entry| entry == "resources.arsc") {
            ResourceTable::parse(&read_entry(&mut archive, "resources.arsc")?).ok()
        } else {
            None
        };
        let resource_status = match &resources {
            Some(table) => format!("{} resources", table.values.len()),
            None if info.entries.iter().any(|entry| entry == "resources.arsc") => {
                "present but not decoded".to_owned()
            }
            None => "not present".to_owned(),
        };
        let launcher = manifest
            .launcher_activity
            .clone()
            .unwrap_or_else(|| "none".to_owned());
        let compatibility = compat::scan_dex(&dex);
        Ok(LaunchReport {
            package: manifest.package,
            launcher_activity: launcher.clone(),
            application_label: resolve_application_label(
                manifest.application_label.as_deref(),
                resources.as_ref(),
            ),
            dex: format!(
                "{} bytes / {} classes / {} methods",
                dex.header.file_size,
                dex.classes.len(),
                dex.methods.len()
            ),
            message: format!(
                "platform: Android 1.x (API 1-4); manifest decoded; launcher: {launcher}; resources: {resource_status}"
            ),
            compatibility,
        })
    }

    pub fn launch(&mut self, path: impl AsRef<Path>) -> Result<LaunchReport> {
        let plan = self.launch_plan(path)?;
        self.game_title = plan
            .application_label
            .clone()
            .or_else(|| Some(plan.package.clone()));
        let state = self.boot(&plan)?;
        self.activities = state.activities;
        let compatibility = compat::scan_dex(&plan.dex);
        Ok(LaunchReport {
            package: plan.package,
            dex: format!(
                "{} bytes / {} classes / {} methods",
                plan.dex.header.file_size,
                plan.dex.classes.len(),
                plan.dex.methods.len()
            ),
            launcher_activity: plan.activity,
            application_label: plan.application_label,
            message: format!("booted launcher; {}", state.graphics),
            compatibility,
        })
    }

    pub fn parse_dex(&self, bytes: &[u8]) -> Result<DexHeader> {
        DexHeader::parse(bytes)
    }

    pub fn parse_manifest(&self, bytes: &[u8]) -> Result<AppManifest> {
        AppManifest::parse_axml(bytes)
    }

    pub fn launch_plan(&self, path: impl AsRef<Path>) -> Result<LaunchPlan> {
        let path = path.as_ref();
        let info = apk::inspect(path)?;
        if !info.has_manifest {
            bail!("APK has no AndroidManifest.xml");
        }
        if !info.has_dex {
            bail!("APK has no classes.dex");
        }
        let file = File::open(path).with_context(|| format!("open APK {}", path.display()))?;
        let mut archive = zip::ZipArchive::new(file)?;
        let manifest = read_manifest(&mut archive)?;
        let dex = read_dex(&mut archive)?;
        let resources = read_resources(&mut archive)?;
        let application_label =
            resolve_application_label(manifest.application_label.as_deref(), resources.as_ref());
        let assets = crate::assets::AssetStore::from_archive(&mut archive)?;
        let activity = manifest
            .launcher_activity
            .clone()
            .ok_or_else(|| anyhow::anyhow!("manifest has no MAIN/LAUNCHER activity"))?;
        let class_name = format!("L{};", activity.replace('.', "/"));
        let entry_method = dex.method_code(&class_name, "onCreate").is_some();
        Ok(LaunchPlan {
            package: manifest.package,
            application_label,
            activity,
            class_name,
            resources,
            assets,
            dex,
            entry_method,
        })
    }

    pub fn boot(&mut self, plan: &LaunchPlan) -> Result<BootState> {
        let mut activities = ActivityManager::default();
        activities.start_activity(
            plan.activity.clone(),
            Intent {
                action: Some("android.intent.action.MAIN".to_owned()),
                categories: vec!["android.intent.category.LAUNCHER".to_owned()],
                component: Some(plan.activity.clone()),
                extras: Default::default(),
            },
        );
        let result = if plan.entry_method {
            let mut framework = std::mem::take(&mut self.framework);
            framework.activities = activities.clone();
            framework.assets = Some(plan.assets.clone());
            if let Some(resources) = &plan.resources {
                for value in &resources.values {
                    framework
                        .resources
                        .insert(value.id, Value::String(value.value.clone()));
                    if value.type_name == "drawable" {
                        if let Some(path) = plan.assets.find_image(&[value.name.as_str()]) {
                            framework.resource_images.insert(value.id, path);
                        }
                    }
                }
            }
            let dex: &'static DexFile = Box::leak(Box::new(plan.dex.clone()));
            let mut vm = Vm::new(
                dex,
                framework,
                VmConfig {
                    max_steps: self.config.max_steps,
                    max_call_depth: 256,
                },
            );
            let method_index = plan
                .dex
                .methods
                .iter()
                .position(|method| {
                    method.class_name == plan.class_name && method.name == "onCreate"
                })
                .ok_or_else(|| anyhow::anyhow!("launcher onCreate method is missing"))?;
            let activity_object = vm.alloc_instance(plan.class_name.clone());
            vm.run_instance_method(activity_object, "<init>", Vec::new())
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
            let value = vm
                .run_method(
                    method_index,
                    vec![VmValue::Object(activity_object), VmValue::Null],
                )
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
            let listener = vm
                .framework
                .gdx_listener
                .or_else(|| vm.find_instance_by_class("Lcom/hyperkani/sliceice/Engine;"));
            let frame_status;
            if let Some(listener) = listener {
                vm.run_instance_method(listener, "create", Vec::new())
                    .map_err(|error| anyhow::anyhow!(error.to_string()))?;
                let mut session = RuntimeSession {
                    vm,
                    listener,
                    render_method: "render".to_owned(),
                };
                let (commands, pixels) = session.render_current_frame()?;
                frame_status = format!(
                    "application create/render completed; GLES commands: {commands}, rendered pixels: {pixels}"
                );
                activities = session.vm.framework.activities.clone();
                self.session = Some(session);
                self.framework = Framework::new();
            } else {
                let listener = vm.find_render_view().ok_or_else(|| {
                    anyhow::anyhow!("application launched but created no renderable View")
                })?;
                let mut session = RuntimeSession {
                    vm,
                    listener,
                    render_method: "onDraw".to_owned(),
                };
                let (commands, pixels) = session.render_current_frame()?;
                frame_status = format!(
                    "legacy Canvas view rendered; GLES commands: {commands}, rendered pixels: {pixels}"
                );
                activities = session.vm.framework.activities.clone();
                self.session = Some(session);
                self.framework = Framework::new();
            }
            return Ok(BootState {
                result: match value {
                    VmValue::Void => ExecutionResult::ReturnVoid,
                    VmValue::Int(value) => ExecutionResult::Return(value),
                    _ => ExecutionResult::Return(0),
                },
                activities,
                graphics: frame_status,
                vm_result: "onCreate completed".to_owned(),
            });
        } else {
            ExecutionResult::ReturnVoid
        };
        Ok(BootState {
            result,
            activities,
            graphics: "no application code executed".to_owned(),
            vm_result: "launcher has no executable onCreate".to_owned(),
        })
    }

    pub fn demo_framework(&mut self) -> FrameworkSnapshot {
        let mut activities = ActivityManager::default();
        activities.start_activity(
            "android.app.Activity".to_owned(),
            Intent {
                action: Some("android.intent.action.MAIN".to_owned()),
                categories: vec!["android.intent.category.LAUNCHER".to_owned()],
                component: Some("android.app.Activity".to_owned()),
                extras: [(
                    "api_level".to_owned(),
                    Value::Int(self.config.api_level as i32),
                )]
                .into_iter()
                .collect(),
            },
        );
        let lifecycle_events = activities.drain_lifecycle().count();
        FrameworkSnapshot {
            activity_count: activities.len(),
            lifecycle_events,
            graphics_commands: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameworkSnapshot {
    pub activity_count: usize,
    pub lifecycle_events: usize,
    pub graphics_commands: usize,
}

#[derive(Debug)]
pub struct LaunchPlan {
    pub package: String,
    pub application_label: Option<String>,
    pub activity: String,
    pub class_name: String,
    pub dex: DexFile,
    pub entry_method: bool,
    pub resources: Option<ResourceTable>,
    pub assets: crate::assets::AssetStore,
}

#[derive(Debug)]
pub struct BootState {
    pub result: ExecutionResult,
    pub activities: ActivityManager,
    pub graphics: String,
    pub vm_result: String,
}

fn resolve_application_label(
    label: Option<&str>,
    resources: Option<&ResourceTable>,
) -> Option<String> {
    let label = label?;
    if let Some(id) = label.strip_prefix("@0x") {
        if let Ok(id) = u32::from_str_radix(id, 16) {
            return resources
                .and_then(|table| table.value_by_id(id))
                .map(str::to_owned);
        }
    }
    Some(label.to_owned())
}

fn read_manifest(archive: &mut zip::ZipArchive<File>) -> Result<AppManifest> {
    let mut entry = archive.by_name("AndroidManifest.xml")?;
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes)?;
    AppManifest::parse_axml(&bytes)
}

fn read_resources(archive: &mut zip::ZipArchive<File>) -> Result<Option<ResourceTable>> {
    let mut entry = match archive.by_name("resources.arsc") {
        Ok(entry) => entry,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes)?;
    Ok(Some(ResourceTable::parse(&bytes)?))
}

fn read_dex(archive: &mut zip::ZipArchive<File>) -> Result<DexFile> {
    let mut entry = archive.by_name("classes.dex")?;
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes)?;
    DexFile::parse(&bytes)
}

fn read_entry<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    name: &str,
) -> Result<Vec<u8>> {
    let mut entry = archive
        .by_name(name)
        .with_context(|| format!("missing APK entry {name}"))?;
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes)?;
    Ok(bytes)
}
