#import <AppKit/AppKit.h>
#import <Sparkle/Sparkle.h>

@interface PinchUpdater : NSObject <SPUUpdaterDelegate, SPUStandardUserDriverDelegate>
@property(nonatomic, strong) SPUStandardUpdaterController *controller;
@property(nonatomic, copy) NSString *availableVersion;
@property(nonatomic) void (*beforeInstall)(void);
@end

@implementation PinchUpdater
- (BOOL)supportsGentleScheduledUpdateReminders { return YES; }
- (BOOL)standardUserDriverShouldHandleShowingScheduledUpdate:(SUAppcastItem *)update
                                       andInImmediateFocus:(BOOL)immediateFocus {
    return NO;
}
- (void)standardUserDriverWillHandleShowingUpdate:(BOOL)handleShowingUpdate
                                      forUpdate:(SUAppcastItem *)update
                                          state:(SPUUserUpdateState *)state {
    self.availableVersion = update.displayVersionString;
}
- (void)standardUserDriverWillFinishUpdateSession {
    self.availableVersion = nil;
}
- (void)updater:(SPUUpdater *)updater willInstallUpdate:(SUAppcastItem *)item {
    if (self.beforeInstall) self.beforeInstall();
}
@end

static PinchUpdater *driver;

// All entry points are called by Pinch's AppKit thread.
bool pinch_updater_start(void (*beforeInstall)(void)) {
    driver = [PinchUpdater new];
    driver.beforeInstall = beforeInstall;
    driver.controller = [[SPUStandardUpdaterController alloc]
        initWithStartingUpdater:NO updaterDelegate:driver userDriverDelegate:driver];
    NSError *error = nil;
    if (![driver.controller.updater startUpdater:&error]) {
        NSLog(@"Pinch updater failed to start: %@", error);
        driver = nil;
        return false;
    }
    return true;
}

unsigned int pinch_updater_status(char *version, size_t capacity) {
    if (!driver) return 0;
    const char *text = driver.availableVersion.UTF8String;
    if (capacity) snprintf(version, capacity, "%s", text ?: "");
    return 1 | (driver.controller.updater.canCheckForUpdates ? 2 : 0)
             | (driver.controller.updater.automaticallyChecksForUpdates ? 4 : 0);
}

void pinch_updater_check(void) {
    [driver.controller checkForUpdates:nil];
}

void pinch_updater_set_automatic(bool enabled) {
    driver.controller.updater.automaticallyChecksForUpdates = enabled;
}
