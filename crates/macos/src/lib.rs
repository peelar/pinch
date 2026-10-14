//! The engine's thin macOS layer: the menu bar status item (shows how many
//! decisions are waiting, opens `pinch-ui` on click) and actionable
//! notifications (Kill / Keep / Snooze) that answer decisions without opening
//! the UI. Built on `objc2`.
//!
//! AppKit objects may only be touched on the main thread, so the rest of the
//! engine never sees them: it talks to this layer through a [`Handle`] (a
//! channel, drained by a timer on the main thread) and hears back through the
//! [`Callbacks`] it registered.

use std::ptr::NonNull;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{AnyThread, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSCellImagePosition, NSImage, NSScreen,
    NSStatusBar, NSStatusBarButton, NSStatusItem, NSVariableStatusItemLength,
};
use objc2_foundation::{
    MainThreadMarker, NSArray, NSBundle, NSObject, NSObjectProtocol, NSSet, NSString, NSTimer,
};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNMutableNotificationContent, UNNotification, UNNotificationAction,
    UNNotificationActionOptions, UNNotificationCategory, UNNotificationCategoryOptions,
    UNNotificationDefaultActionIdentifier, UNNotificationPresentationOptions,
    UNNotificationRequest, UNNotificationResponse, UNNotificationSound, UNUserNotificationCenter,
    UNUserNotificationCenterDelegate,
};

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
    Snooze,
}

/// Called from the main thread (clicks) or an arbitrary one (notification
/// buttons), so keep them quick: hand the work off and return.
pub struct Callbacks {
    pub on_click: Box<dyn Fn(Option<Anchor>) + Send + Sync>,
    pub on_action: Box<dyn Fn(String, Action) + Send + Sync>,
}

enum Command {
    Badge(usize),
    Notify {
        id: String,
        title: String,
        body: String,
    },
}

/// How the rest of the engine talks to the menu bar. Cheap to clone, usable
/// from any thread.
#[derive(Clone)]
pub struct Handle {
    tx: Sender<Command>,
}

impl Handle {
    /// Show `n` next to the icon; 0 clears it.
    pub fn set_badge(&self, n: usize) {
        let _ = self.tx.send(Command::Badge(n));
    }

    /// Post a notification with Kill / Keep / Snooze buttons. `id` comes back
    /// through [`Callbacks::on_action`].
    pub fn notify(&self, id: &str, title: &str, body: &str) {
        let _ = self.tx.send(Command::Notify {
            id: id.into(),
            title: title.into(),
            body: body.into(),
        });
    }
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
                    "snooze" => (callbacks.on_action)(id, Action::Snooze),
                    _ if &*action == default => OPEN_REQUESTED.store(true, Ordering::SeqCst),
                    _ => {}
                }
            }
            completion.call(());
        }
    }
);

/// Register the Kill/Keep/Snooze category and ask for permission. Only works
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
        &ns("Kill"),
        UNNotificationActionOptions::Destructive,
    );
    let keep = UNNotificationAction::actionWithIdentifier_title_options(
        &ns("keep"),
        &ns("Keep"),
        UNNotificationActionOptions::empty(),
    );
    let snooze = UNNotificationAction::actionWithIdentifier_title_options(
        &ns("snooze"),
        &ns("Snooze 1h"),
        UNNotificationActionOptions::empty(),
    );
    let category = UNNotificationCategory::categoryWithIdentifier_actions_intentIdentifiers_options(
        &ns(CATEGORY),
        &NSArray::from_retained_slice(&[kill, keep, snooze]),
        &NSArray::new(),
        UNNotificationCategoryOptions::empty(),
    );
    center.setNotificationCategories(&NSSet::from_retained_slice(&[category]));

    let done = RcBlock::new(
        |granted: objc2::runtime::Bool, _err: *mut objc2_foundation::NSError| {
            if !granted.as_bool() {
                eprintln!("pinchd: notification permission was not granted");
            }
        },
    );
    center.requestAuthorizationWithOptions_completionHandler(
        UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
        &done,
    );
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
    center.addNotificationRequest_withCompletionHandler(&request, None);
}

fn set_badge(button: &NSStatusBarButton, n: usize) {
    let title = if n == 0 {
        String::new()
    } else {
        format!(" {n}")
    };
    button.setTitle(&ns(&title));
}

/// Run the menu bar app on the calling thread, which must be the main thread.
/// `on_ready` runs on a new thread once the item exists and receives the
/// [`Handle`] to drive it. Never returns.
pub fn run(callbacks: Callbacks, on_ready: impl FnOnce(Handle) + Send + 'static) -> ! {
    let mtm = MainThreadMarker::new().expect("pinch_macos::run must be called on the main thread");
    let _ = CALLBACKS.set(callbacks);

    let app = NSApplication::sharedApplication(mtm);
    // Accessory = no Dock icon and no app menu: a pure menu bar app.
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

    let status_item: Retained<NSStatusItem> =
        NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
    let target: Retained<ClickTarget> = unsafe { msg_send![ClickTarget::alloc(mtm), init] };
    let button = status_item.button(mtm).expect("status item has a button");
    match NSImage::imageWithSystemSymbolName_accessibilityDescription(
        &ns("gauge.with.dots.needle.33percent"),
        Some(&ns("Pinch")),
    ) {
        Some(image) => {
            image.setTemplate(true);
            button.setImage(Some(&image));
            button.setImagePosition(NSCellImagePosition::ImageLeft);
        }
        None => button.setTitle(&ns("◐")),
    }
    unsafe {
        button.setTarget(Some(&target));
        button.setAction(Some(sel!(clicked:)));
    }

    let _delegate = setup_notifications();
    let notifications = _delegate.is_some();

    let (tx, rx) = mpsc::channel::<Command>();
    let timer_button = button.clone();
    let tick = RcBlock::new(move |_timer: NonNull<NSTimer>| {
        while let Ok(command) = rx.try_recv() {
            match command {
                Command::Badge(n) => set_badge(&timer_button, n),
                Command::Notify { id, title, body } if notifications => post(&id, &title, &body),
                Command::Notify { .. } => {}
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
