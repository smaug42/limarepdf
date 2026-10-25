//! macOS: Finder's Open With, a double-click and a file dropped on the Dock icon send the app
//! an "open documents" Apple Event, which this turns into files for incoming.rs (BUG-28).
//!
//! The usual way in is the application delegate, but winit 0.30 keeps the delegate for itself
//! and panics if another replaces it (winit issues 4015 and 4260). So the event is taken from
//! the Apple Event manager instead: AppKit installs its own handler as launching finishes (one
//! that only opens files through a delegate or document controller, which this app has
//! neither of), and ours replaces it when launching is about to finish, where Apple says to,
//! before the first event is handled. Events can come during launch or long after it (with
//! the macOS 26 SDK, launch's own can come after it has finished), and every one is handed in.

use crate::incoming::Incoming;
use objc2::rc::Retained;
use objc2::{define_class, msg_send, sel, AnyThread};
use objc2_foundation::{
    ns_string, NSAppleEventDescriptor, NSAppleEventManager, NSNotification, NSNotificationCenter,
    NSObject,
};
use std::path::PathBuf;

// Four-character codes from Apple's AppleEvents.h and AEDataModel.h.
/// kCoreEventClass
const CORE_EVENT_CLASS: u32 = u32::from_be_bytes(*b"aevt");
/// kAEOpenDocuments
const OPEN_DOCUMENTS: u32 = u32::from_be_bytes(*b"odoc");
/// keyDirectObject: the event's files.
const DIRECT_OBJECT: u32 = u32::from_be_bytes(*b"----");
/// typeAEList
const LIST: u32 = u32::from_be_bytes(*b"list");

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and `Opener` doesn't implement Drop.
    #[unsafe(super(NSObject))]
    #[name = "LimarePDFOpener"]
    struct Opener;

    impl Opener {
        #[unsafe(method(willFinishLaunching:))]
        fn will_finish_launching(&self, _note: &NSNotification) {
            let manager = NSAppleEventManager::sharedAppleEventManager();
            // SAFETY: `self` lives as long as the app (`listen`) and answers the selector, which
            // takes the event and its reply.
            let _: () = unsafe {
                msg_send![
                    &*manager,
                    setEventHandler: self,
                    andSelector: sel!(openDocuments:withReplyEvent:),
                    forEventClass: CORE_EVENT_CLASS,
                    andEventID: OPEN_DOCUMENTS
                ]
            };
        }

        #[unsafe(method(openDocuments:withReplyEvent:))]
        fn open_documents(&self, event: &NSAppleEventDescriptor, _reply: &NSAppleEventDescriptor) {
            let (files, unread) = files_in(event);
            Incoming::system().hand_in(files, unread);
        }
    }
);

/// Listens for the files Finder sends. Called before the event loop starts, so that the one
/// sent as the app launches isn't missed.
pub fn listen() {
    // SAFETY: NSObject's init, on an `Opener`, which has no fields of its own.
    let opener: Retained<Opener> = unsafe { msg_send![super(Opener::alloc().set_ivars(())), init] };
    // NSApplicationWillFinishLaunchingNotification's value: the constant is AppKit's.
    let name = ns_string!("NSApplicationWillFinishLaunchingNotification");
    // SAFETY: `opener` answers the selector, which takes the notification.
    unsafe {
        NSNotificationCenter::defaultCenter().addObserver_selector_name_object(
            &opener,
            sel!(willFinishLaunching:),
            Some(name),
            None,
        );
    }
    // Neither the notification centre nor the Apple Event manager keeps a handler alive: this
    // one lives as long as the app.
    std::mem::forget(opener);
}

/// The files an "open documents" event names, and how many of its items aren't files on disk.
fn files_in(event: &NSAppleEventDescriptor) -> (Vec<PathBuf>, usize) {
    // SAFETY: the keyword is a four-character code, and the answer may be nil.
    let items: Option<Retained<NSAppleEventDescriptor>> =
        unsafe { msg_send![event, paramDescriptorForKeyword: DIRECT_OBJECT] };
    let Some(items) = items else {
        return (Vec::new(), 1);
    };
    // SAFETY: a descriptor's type is a four-character code.
    let kind: u32 = unsafe { msg_send![&*items, descriptorType] };
    // Finder sends a list; a single item is read as a list of one.
    let each: Vec<Option<Retained<NSAppleEventDescriptor>>> = if kind == LIST {
        (1..=items.numberOfItems())
            .map(|i| items.descriptorAtIndex(i))
            .collect()
    } else {
        vec![Some(items)]
    };
    let mut files = Vec::new();
    let mut unread = 0;
    for item in each {
        // A file URL, coerced from an alias or bookmark if need be.
        match item
            .and_then(|d| d.fileURLValue())
            .and_then(|u| u.to_file_path())
        {
            Some(p) => files.push(p),
            None => unread += 1,
        }
    }
    (files, unread)
}
