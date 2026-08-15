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
    gles::GlesContext,
    manifest::AppManifest,
    resources::ResourceTable,
    vm::{Value as VmValue, Vm, VmConfig},
    VirtualScreen,
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
            api_level: 4,
            release: "Donut",
            max_steps: 100_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchReport {
    pub package: String,
    pub dex: String,
    pub launcher_activity: String,
    pub message: String,
    pub compatibility: CompatibilityReport,
}

pub struct Runtime {
    pub config: RuntimeConfig,
    pub activities: ActivityManager,
    pub graphics: GlesContext,
    pub framework: Framework,
}

impl Default for Runtime {
    fn default() -> Self {
        let config = RuntimeConfig::default();
        Self {
            graphics: GlesContext::new(config.screen),
            config,
            activities: ActivityManager::default(),
            framework: Framework::new(),
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
        let resource_status = if info.entries.iter().any(|entry| entry == "resources.arsc") {
            match ResourceTable::parse(&read_entry(&mut archive, "resources.arsc")?) {
                Ok(table) => format!("{} resources", table.values.len()),
                Err(_) => "present but not decoded".to_owned(),
            }
        } else {
            "not present".to_owned()
        };
        let launcher = manifest
            .launcher_activity
            .clone()
            .unwrap_or_else(|| "none".to_owned());
        let compatibility = compat::scan_dex(&dex);
        Ok(LaunchReport {
            package: manifest.package,
            launcher_activity: launcher.clone(),
            dex: format!(
                "{} bytes / {} classes / {} methods",
                dex.header.file_size,
                dex.classes.len(),
                dex.methods.len()
            ),
            message: format!(
                "manifest decoded; launcher: {launcher}; resources: {resource_status}"
            ),
            compatibility,
        })
    }

    pub fn launch(&mut self, path: impl AsRef<Path>) -> Result<LaunchReport> {
        let plan = self.launch_plan(path)?;
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
        let activity = manifest
            .launcher_activity
            .clone()
            .ok_or_else(|| anyhow::anyhow!("manifest has no MAIN/LAUNCHER activity"))?;
        let class_name = format!("L{};", activity.replace('.', "/"));
        let entry_method = dex.method_code(&class_name, "onCreate").is_some();
        Ok(LaunchPlan {
            package: manifest.package,
            activity,
            class_name,
            resources: read_resources(&mut archive)?,
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
            framework.activities = activities;
            if let Some(resources) = &plan.resources {
                for value in &resources.values {
                    framework
                        .resources
                        .insert(value.id, Value::String(value.value.clone()));
                }
            }
            let mut vm = Vm::new(
                &plan.dex,
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
            let value = vm
                .run_method(
                    method_index,
                    vec![VmValue::Object(activity_object), VmValue::Null],
                )
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
            self.framework = vm.framework;
            activities = std::mem::take(&mut self.framework.activities);
            return Ok(BootState {
                result: match value {
                    VmValue::Void => ExecutionResult::ReturnVoid,
                    VmValue::Int(value) => ExecutionResult::Return(value),
                    _ => ExecutionResult::Return(0),
                },
                activities,
                graphics: "launcher onCreate completed; application render loop deferred"
                    .to_owned(),
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
    pub activity: String,
    pub class_name: String,
    pub dex: DexFile,
    pub entry_method: bool,
    pub resources: Option<ResourceTable>,
}

#[derive(Debug)]
pub struct BootState {
    pub result: ExecutionResult,
    pub activities: ActivityManager,
    pub graphics: String,
    pub vm_result: String,
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
