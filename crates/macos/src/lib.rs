//! The engine's thin macOS layer: the menu bar status item (shows how many
//! decisions are waiting, opens `pinch-ui` on click) and actionable
//! notifications (Stop / Keep running) that answer decisions without opening
//! the UI. Built on `objc2`.
//!
//! AppKit objects may only be touched on the main thread, so the rest of the
//! engine never sees them: it talks to this layer through a [`Handle`] (a
//! channel, drained by a timer on the main thread) and hears back through the
//! [`Callbacks`] it registered.

pub mod updater;

use std::collections::HashSet;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Mutex, OnceLock};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{AnyThread, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSCellImagePosition, NSImage, NSScreen,
    NSStatusBar, NSStatusBarButton, NSStatusItem, NSVariableStatusItemLength,
};
use objc2_foundation::{
    MainThreadMarker, NSArray, NSBundle, NSData, NSObject, NSObjectProtocol, NSSet, NSSize,
    NSString, NSTimer,
};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNAuthorizationStatus, UNMutableNotificationContent, UNNotification,
    UNNotificationAction, UNNotificationActionOptions, UNNotificationCategory,
    UNNotificationCategoryOptions, UNNotificationDefaultActionIdentifier,
    UNNotificationPresentationOptions, UNNotificationRequest, UNNotificationResponse,
    UNNotificationSetting, UNNotificationSettings, UNNotificationSound, UNUserNotificationCenter,
    UNUserNotificationCenterDelegate,
};
use pinch_proto::{Decision, NotificationStatus};

/// Where the menu bar icon sits, in the top-left-origin coordinates GPUI uses
/// (points): the popover opens centred under `x_center`, just below `y_bottom`.
#[derive(Debug, Clone, Copy)]
pub struct Anchor {
    pub x_center: f64,
    pub y_bottom: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Kill,
    Keep,
}

/// Called from the main thread (clicks) or an arbitrary one (notification
/// buttons), so keep them quick: hand the work off and return.
pub struct Callbacks {
    pub on_update_install: Box<dyn Fn() + Send + Sync>,
    pub on_click: Box<dyn Fn(Option<Anchor>) + Send + Sync>,
    pub on_action: Box<dyn Fn(String, Action) + Send + Sync>,
}

enum Command {
    Decisions {
        decisions: Vec<Decision>,
        restored: bool,
    },
}

#[derive(Clone)]
pub struct Handle {
    tx: Sender<Command>,
}

impl Handle {
    pub fn sync_decisions(&self, decisions: &[Decision], restored: bool) {
        let _ = self.tx.send(Command::Decisions {
            decisions: decisions.to_vec(),
            restored,
        });
    }
}

static NOTIFICATION_STATUS: AtomicU8 = AtomicU8::new(0);
static ACTIVE_DECISIONS: Mutex<Option<HashSet<String>>> = Mutex::new(None);

pub fn notification_status() -> NotificationStatus {
    match NOTIFICATION_STATUS.load(Ordering::Relaxed) {
        1 => NotificationStatus::NotDetermined,
        2 => NotificationStatus::Disabled,
        3 => NotificationStatus::Enabled,
        _ => NotificationStatus::Unavailable,
    }
}

fn refresh_notification_status() {
    let done = RcBlock::new(|settings: NonNull<UNNotificationSettings>| {
        let settings = unsafe { settings.as_ref() };
        let status = if settings.authorizationStatus() == UNAuthorizationStatus::NotDetermined {
            1
        } else if settings.authorizationStatus() == UNAuthorizationStatus::Authorized
            && settings.alertSetting() == UNNotificationSetting::Enabled
        {
            3
        } else {
            2
        };
        NOTIFICATION_STATUS.store(status, Ordering::Relaxed);
    });
    UNUserNotificationCenter::currentNotificationCenter()
        .getNotificationSettingsWithCompletionHandler(&done);
}

fn remove_notifications(ids: &[String]) {
    let ids: Vec<_> = ids.iter().map(|id| ns(id)).collect();
    let ids = NSArray::from_retained_slice(&ids);
    let center = UNUserNotificationCenter::currentNotificationCenter();
    center.removePendingNotificationRequestsWithIdentifiers(&ids);
    center.removeDeliveredNotificationsWithIdentifiers(&ids);
}

fn remove_orphaned_notifications() {
    let done = RcBlock::new(|notifications: NonNull<NSArray<UNNotification>>| {
        let notifications = unsafe { notifications.as_ref() };
        let active = ACTIVE_DECISIONS.lock().unwrap_or_else(|e| e.into_inner());
        let ids = notifications
            .iter()
            .map(|n| n.request().identifier().to_string())
            .filter(|id| !active.as_ref().is_some_and(|ids| ids.contains(id)))
            .collect::<Vec<_>>();
        remove_notifications(&ids);
    });
    UNUserNotificationCenter::currentNotificationCenter()
        .getDeliveredNotificationsWithCompletionHandler(&done);
}

static CALLBACKS: OnceLock<Callbacks> = OnceLock::new();
/// Set when the user taps a notification itself; the main thread then opens the UI.
static OPEN_REQUESTED: AtomicBool = AtomicBool::new(false);

const CATEGORY: &str = "pinch.decision";

fn ns(s: &str) -> Retained<NSString> {
    NSString::from_str(s)
}

/// The icon's rectangle in top-left coordinates, from its button.
fn anchor_of(button: &NSStatusBarButton, mtm: MainThreadMarker) -> Option<Anchor> {
    let frame = button.window()?.frame();
    // Screen 0 is the one with the menu bar; Cocoa's origin is its bottom-left.
    let screen_height = NSScreen::screens(mtm).firstObject()?.frame().size.height;
    Some(Anchor {
        x_center: frame.origin.x + frame.size.width / 2.0,
        y_bottom: screen_height - frame.origin.y,
    })
}

define_class!(
    // Objective-C classes can't hold Rust closures, so this one only receives
    // the click and forwards it to the registered callback.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    struct ClickTarget;

    impl ClickTarget {
        #[unsafe(method(clicked:))]
        fn clicked(&self, sender: &NSStatusBarButton) {
            let anchor = anchor_of(sender, self.mtm());
            if let Some(callbacks) = CALLBACKS.get() {
                (callbacks.on_click)(anchor);
            }
        }
    }

    unsafe impl NSObjectProtocol for ClickTarget {}
);

define_class!(
    // Notification delegate. macOS may call it on any thread, so it is not
    // main-thread-only and touches nothing but the callbacks and a flag.
    #[unsafe(super(NSObject))]
    struct NotificationDelegate;

    unsafe impl NSObjectProtocol for NotificationDelegate {}

    unsafe impl UNUserNotificationCenterDelegate for NotificationDelegate {
        // Show banners even while we're the frontmost app (we never are, but
        // accessory apps are treated as such sometimes).
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(
            &self,
            _center: &UNUserNotificationCenter,
            _notification: &UNNotification,
            completion: &block2::DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            completion.call((UNNotificationPresentationOptions::Banner
                | UNNotificationPresentationOptions::List
                | UNNotificationPresentationOptions::Sound,));
        }

        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn did_receive(
            &self,
            _center: &UNUserNotificationCenter,
            response: &UNNotificationResponse,
            completion: &block2::DynBlock<dyn Fn()>,
        ) {
            let action = response.actionIdentifier();
            let id = response.notification().request().identifier().to_string();
            // SAFETY: a static NSString constant provided by UserNotifications.
            let default = unsafe { UNNotificationDefaultActionIdentifier };
            if let Some(callbacks) = CALLBACKS.get() {
                match action.to_string().as_str() {
                    "kill" => (callbacks.on_action)(id, Action::Kill),
                    "keep" => (callbacks.on_action)(id, Action::Keep),
                    _ if &*action == default => OPEN_REQUESTED.store(true, Ordering::SeqCst),
                    _ => {}
                }
            }
            completion.call(());
        }
    }
);

/// Register the Kill/Keep category and ask for permission. Only works
/// for a process running from a `.app` bundle; returns whether it did.
fn setup_notifications() -> Option<Retained<NotificationDelegate>> {
    if NSBundle::mainBundle().bundleIdentifier().is_none() {
        eprintln!("pinchd: not running from Pinch.app, so notifications are disabled");
        return None;
    }
    let center = UNUserNotificationCenter::currentNotificationCenter();

    let delegate: Retained<NotificationDelegate> =
        unsafe { msg_send![NotificationDelegate::alloc(), init] };
    center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));

    let kill = UNNotificationAction::actionWithIdentifier_title_options(
        &ns("kill"),
        &ns("Stop"),
        UNNotificationActionOptions::Destructive,
    );
    let keep = UNNotificationAction::actionWithIdentifier_title_options(
        &ns("keep"),
        &ns("Keep running"),
        UNNotificationActionOptions::empty(),
    );
    let category = UNNotificationCategory::categoryWithIdentifier_actions_intentIdentifiers_options(
        &ns(CATEGORY),
        &NSArray::from_retained_slice(&[kill, keep]),
        &NSArray::new(),
        UNNotificationCategoryOptions::empty(),
    );
    center.setNotificationCategories(&NSSet::from_retained_slice(&[category]));

    let done = RcBlock::new(
        |granted: objc2::runtime::Bool, _err: *mut objc2_foundation::NSError| {
            if !granted.as_bool() {
                eprintln!("pinchd: notification permission was not granted");
            }
            refresh_notification_status();
        },
    );
    center.requestAuthorizationWithOptions_completionHandler(
        UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
        &done,
    );
    refresh_notification_status();
    Some(delegate)
}

fn post(id: &str, title: &str, body: &str) {
    let center = UNUserNotificationCenter::currentNotificationCenter();
    let content = UNMutableNotificationContent::new();
    content.setTitle(&ns(title));
    content.setBody(&ns(body));
    content.setCategoryIdentifier(&ns(CATEGORY));
    content.setSound(Some(&UNNotificationSound::defaultSound()));
    let request =
        UNNotificationRequest::requestWithIdentifier_content_trigger(&ns(id), &content, None);
    let done = RcBlock::new(|error: *mut objc2_foundation::NSError| {
        if let Some(error) = unsafe { error.as_ref() } {
            eprintln!("pinchd: notification delivery failed: {error}");
        }
    });
    center.addNotificationRequest_withCompletionHandler(&request, Some(&done));
}

fn set_badge(button: &NSStatusBarButton, n: usize, variant: Option<&str>) {
    let title = if n == 0 {
        String::new()
    } else {
        format!(" {n}")
    };
    let title = match variant {
        Some(variant) => format!(" {variant}{title}"),
        None => title,
    };
    button.setTitle(&ns(&title));
}

/// Run the menu bar app on the calling thread, which must be the main thread.
/// `on_ready` runs on a new thread once the item exists and receives the
/// [`Handle`] to drive it. Never returns.
pub fn run(
    callbacks: Callbacks,
    variant: Option<String>,
    on_ready: impl FnOnce(Handle) + Send + 'static,
) -> ! {
    let mtm = MainThreadMarker::new().expect("pinch_macos::run must be called on the main thread");
    let _ = CALLBACKS.set(callbacks);

    let app = NSApplication::sharedApplication(mtm);
    // Accessory = no Dock icon and no app menu: a pure menu bar app.
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

    let status_item: Retained<NSStatusItem> =
        NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
    let target: Retained<ClickTarget> = unsafe { msg_send![ClickTarget::alloc(mtm), init] };
    let button = status_item.button(mtm).expect("status item has a button");
    let data = NSData::with_bytes(include_bytes!("../../../assets/brand/menu-bar.png"));
    if let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) {
        image.setSize(NSSize::new(24.0, 16.0));
        image.setTemplate(true);
        button.setImage(Some(&image));
        button.setImagePosition(NSCellImagePosition::ImageLeft);
    }
    button.setToolTip(Some(&ns("Pinch — local servers under control")));
    unsafe {
        button.setTarget(Some(&target));
        button.setAction(Some(sel!(clicked:)));
    }

    set_badge(&button, 0, variant.as_deref());
    let _delegate = setup_notifications();
    let notifications = _delegate.is_some();
    updater::start();

    let (tx, rx) = mpsc::channel::<Command>();
    let timer_button = button.clone();
    struct NotificationState {
        decisions: Vec<Decision>,
        notified: HashSet<String>,
        last_settings_check: std::time::Instant,
    }
    let state = std::cell::RefCell::new(NotificationState {
        decisions: Vec::new(),
        notified: HashSet::new(),
        last_settings_check: std::time::Instant::now(),
    });
    let tick = RcBlock::new(move |_timer: NonNull<NSTimer>| {
        updater::poll();
        let mut state = state.borrow_mut();
        let NotificationState {
            decisions,
            notified,
            last_settings_check,
        } = &mut *state;
        while let Ok(command) = rx.try_recv() {
            match command {
                Command::Decisions {
                    decisions: next,
                    restored,
                } => {
                    let ids: HashSet<_> = next.iter().map(|d| d.id.clone()).collect();
                    let removed: Vec<_> = decisions
                        .iter()
                        .filter(|d| !ids.contains(&d.id))
                        .map(|d| d.id.clone())
                        .collect();
                    if notifications {
                        remove_notifications(&removed);
                    }
                    notified.retain(|id| ids.contains(id));
                    *ACTIVE_DECISIONS.lock().unwrap_or_else(|e| e.into_inner()) = Some(ids.clone());
                    if restored {
                        notified.extend(ids);
                        if notifications {
                            remove_orphaned_notifications();
                        }
                    }
                    *decisions = next;
                    set_badge(&timer_button, decisions.len(), variant.as_deref());
                }
            }
        }
        if notifications {
            if last_settings_check.elapsed().as_secs() >= 10 {
                refresh_notification_status();
                *last_settings_check = std::time::Instant::now();
            }
            if notification_status() == NotificationStatus::Enabled {
                for decision in decisions.iter() {
                    if notified.insert(decision.id.clone()) {
                        post(
                            &decision.id,
                            &format!("Still need {}?", decision.process_name),
                            &decision.reason,
                        );
                    }
                }
            }
        }
        if OPEN_REQUESTED.swap(false, Ordering::SeqCst)
            && let Some(callbacks) = CALLBACKS.get()
        {
            (callbacks.on_click)(anchor_of(&timer_button, mtm));
        }
    });
    let _timer = unsafe { NSTimer::scheduledTimerWithTimeInterval_repeats_block(0.2, true, &tick) };

    std::thread::spawn(move || on_ready(Handle { tx }));
    app.run();
    std::process::exit(0)
}
