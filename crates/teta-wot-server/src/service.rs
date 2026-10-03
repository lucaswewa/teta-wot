//! Running as a Windows service (feature `windows-service`).
//!
//! [`run_as_service`] connects the process to the Service Control Manager
//! (SCM) and runs the server there: the SCM's *stop* and *shutdown*
//! requests become the server's shutdown signal, so a service stops the way
//! Ctrl-C stops a console server:
//!
//! 1. the SCM starts the process; it reports *start pending*, then
//!    *running* once `serve` is called;
//! 2. on *stop* or *shutdown*, it reports *stop pending*, with a wait hint
//!    that covers the grace period, and the shutdown future completes;
//! 3. when `serve` returns, it reports *stopped*, with its exit code.
//!
//! If the SCM didn't start the process, [`run_as_service`] returns
//! [`ServiceError::NotAService`] at once, so the same binary can run in a
//! console. [`install`] and [`uninstall`] register the binary with the SCM,
//! which needs an administrator.

use std::ffi::OsString;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::process::ExitCode;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use windows_service::service::{
    ServiceAccess, ServiceControl, ServiceControlAccept, ServiceErrorControl, ServiceExitCode,
    ServiceInfo, ServiceStartType, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
use windows_service::{define_windows_service, service_dispatcher};

/// The Win32 error when a process that the SCM didn't start calls
/// `StartServiceCtrlDispatcher` (`ERROR_FAILED_SERVICE_CONTROLLER_CONNECT`).
const NOT_A_SERVICE: i32 = 1063;

/// How long the SCM is told a start takes at most.
const START_HINT: Duration = Duration::from_secs(30);

/// Running as a service, or registering one, failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ServiceError {
    /// The process wasn't started by the Service Control Manager: run it
    /// in a console instead.
    #[error("the process wasn't started by the Service Control Manager")]
    NotAService,
    /// [`run_as_service`] was called twice in one process.
    #[error("the process is already running as a service")]
    AlreadyRunning,
    /// The SCM refused a request, for example because installing needs an
    /// administrator.
    #[error("the Service Control Manager refused: {0}")]
    Scm(#[from] windows_service::Error),
    /// The service's runtime couldn't be built.
    #[error("the service's runtime couldn't start: {0}")]
    Runtime(#[from] std::io::Error),
}

/// The future that completes when the SCM asks the service to stop.
pub type ServiceStop = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

type Serve =
    Box<dyn FnOnce(Vec<OsString>, ServiceStop) -> Pin<Box<dyn Future<Output = ExitCode>>> + Send>;

/// What the SCM's thread runs: the name, the stop wait hint, and `serve`.
struct Service {
    name: String,
    stop_hint: Duration,
    serve: Serve,
}

static SERVICE: OnceLock<Mutex<Option<Service>>> = OnceLock::new();

define_windows_service!(ffi_service_main, service_main);

/// Runs the process as the Windows service `name`, calling `serve` with the
/// service's start arguments and its stop signal, on a multi-threaded tokio
/// runtime. `stop_hint` tells the SCM how long stopping may take: the
/// shutdown grace period, plus the time to stop the Things.
///
/// Returns once the service has stopped, or at once with
/// [`ServiceError::NotAService`] if the SCM didn't start the process.
///
/// ```no_run
/// use teta_wot_server::service::{ServiceError, run_as_service};
/// use teta_wot_server::cli;
/// # fn registry() -> teta_wot_server::ThingRegistry { teta_wot_server::ThingRegistry::new() }
///
/// fn main() -> std::process::ExitCode {
///     let args: Vec<std::ffi::OsString> = std::env::args_os().collect();
///     let service = run_as_service("microscope", std::time::Duration::from_secs(20), move |_, stop| {
///         Box::pin(async move { cli::serve_until(args, &registry(), |b| b, stop).await })
///     });
///     match service {
///         Ok(()) => std::process::ExitCode::SUCCESS,
///         Err(ServiceError::NotAService) => std::process::ExitCode::from(2), // run in a console instead
///         Err(_) => std::process::ExitCode::FAILURE,
///     }
/// }
/// ```
pub fn run_as_service<F>(name: &str, stop_hint: Duration, serve: F) -> Result<(), ServiceError>
where
    F: FnOnce(Vec<OsString>, ServiceStop) -> Pin<Box<dyn Future<Output = ExitCode>>>
        + Send
        + 'static,
{
    let slot = SERVICE.get_or_init(|| Mutex::new(None));
    {
        let mut slot = slot.lock().unwrap_or_else(|e| e.into_inner());
        if slot.is_some() {
            return Err(ServiceError::AlreadyRunning);
        }
        *slot = Some(Service {
            name: name.to_owned(),
            stop_hint,
            serve: Box::new(serve),
        });
    }
    // Blocks until the service has stopped; the SCM calls `service_main`
    // on a thread of its own.
    match service_dispatcher::start(name, ffi_service_main) {
        Ok(()) => Ok(()),
        Err(windows_service::Error::Winapi(error))
            if error.raw_os_error() == Some(NOT_A_SERVICE) =>
        {
            *slot.lock().unwrap_or_else(|e| e.into_inner()) = None;
            Err(ServiceError::NotAService)
        }
        Err(error) => Err(error.into()),
    }
}

/// The service's entry point, on the SCM's thread.
fn service_main(arguments: Vec<OsString>) {
    let service = SERVICE
        .get()
        .and_then(|slot| slot.lock().unwrap_or_else(|e| e.into_inner()).take());
    let Some(service) = service else {
        return;
    };
    if let Err(error) = run(service, arguments) {
        tracing::error!("the service failed: {error}");
    }
}

fn run(service: Service, arguments: Vec<OsString>) -> Result<(), ServiceError> {
    let (stop, stopped) = tokio::sync::watch::channel(false);
    let stop_hint = service.stop_hint;
    let status = service_control_handler::register(&service.name, {
        let stop = stop.clone();
        move |control| match control {
            ServiceControl::Stop | ServiceControl::Shutdown | ServiceControl::Preshutdown => {
                tracing::info!("the Service Control Manager asked the service to stop");
                let _ = stop.send(true);
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        }
    })?;
    let report = move |state: ServiceState, wait_hint: Duration, exit: u32| {
        let accepted = if state == ServiceState::Running {
            ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN
        } else {
            ServiceControlAccept::empty()
        };
        status.set_service_status(ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: state,
            controls_accepted: accepted,
            exit_code: if exit == 0 {
                ServiceExitCode::Win32(0)
            } else {
                ServiceExitCode::ServiceSpecific(exit)
            },
            checkpoint: 0,
            wait_hint,
            process_id: None,
        })
    };
    report(ServiceState::StartPending, START_HINT, 0)?;

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let mut watch = stopped.clone();
    // Reports *stop pending* as soon as the request arrives. `report` is
    // `Copy`, so the task gets its own.
    let pending = runtime.spawn(async move {
        if watch.wait_for(|s| *s).await.is_ok() {
            let _ = report(ServiceState::StopPending, stop_hint, 0);
        }
    });
    report(ServiceState::Running, Duration::ZERO, 0)?;
    let mut stopped = stopped;
    let stop_signal: ServiceStop = Box::pin(async move {
        let _ = stopped.wait_for(|s| *s).await;
    });
    let code = runtime.block_on((service.serve)(arguments, stop_signal));
    pending.abort();
    drop(runtime);
    let exit = (0..=255u8)
        .find(|n| ExitCode::from(*n) == code)
        .map_or(1, u32::from);
    tracing::info!("the service stopped with exit code {exit}");
    report(ServiceState::Stopped, Duration::ZERO, exit)?;
    Ok(())
}

/// A service to register with the SCM.
#[derive(Debug, Clone)]
pub struct InstallOptions {
    /// The service's name, as `sc` and `Start-Service` know it.
    pub name: String,
    /// The name shown in the Services console.
    pub display_name: String,
    /// The description shown in the Services console.
    pub description: String,
    /// The executable, which must call [`run_as_service`] with `name`.
    pub executable: PathBuf,
    /// The arguments the SCM starts it with.
    pub arguments: Vec<OsString>,
    /// Whether it starts with Windows (otherwise on demand).
    pub automatic: bool,
}

/// Registers a service with the SCM, running as the LocalSystem account.
/// Needs an administrator.
pub fn install(options: &InstallOptions) -> Result<(), ServiceError> {
    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
    )?;
    let info = ServiceInfo {
        name: OsString::from(&options.name),
        display_name: OsString::from(&options.display_name),
        service_type: ServiceType::OWN_PROCESS,
        start_type: if options.automatic {
            ServiceStartType::AutoStart
        } else {
            ServiceStartType::OnDemand
        },
        error_control: ServiceErrorControl::Normal,
        executable_path: options.executable.clone(),
        launch_arguments: options.arguments.clone(),
        dependencies: Vec::new(),
        account_name: None,
        account_password: None,
    };
    let service = manager.create_service(&info, ServiceAccess::CHANGE_CONFIG)?;
    service.set_description(&options.description)?;
    Ok(())
}

/// Stops a service if it is running, and removes it from the SCM. Needs an
/// administrator.
pub fn uninstall(name: &str) -> Result<(), ServiceError> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)?;
    let service = manager.open_service(
        name,
        ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE,
    )?;
    if service.query_status()?.current_state != ServiceState::Stopped {
        let _ = service.stop();
    }
    service.delete()?;
    Ok(())
}
