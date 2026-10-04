#import "DFRemoteVirtualDisplay.h"

// Local ABI declarations only: no link-time dependency on private classes.
@interface NSObject (DFVirtualDisplayABI)
- (void)setName:(NSString *)name;
- (void)setQueue:(dispatch_queue_t)queue;
- (void)setMaxPixelsWide:(unsigned int)value;
- (void)setMaxPixelsHigh:(unsigned int)value;
- (void)setSizeInMillimeters:(CGSize)value;
- (void)setRedPrimary:(CGPoint)value;
- (void)setGreenPrimary:(CGPoint)value;
- (void)setBluePrimary:(CGPoint)value;
- (void)setWhitePoint:(CGPoint)value;
- (void)setVendorID:(unsigned int)value;
- (void)setProductID:(unsigned int)value;
- (void)setSerialNum:(unsigned int)value;
- (void)setHiDPI:(unsigned int)value;
- (void)setModes:(NSArray *)modes;
- (instancetype)initWithWidth:(unsigned int)width height:(unsigned int)height refreshRate:(double)rate;
- (instancetype)initWithDescriptor:(id)descriptor;
- (BOOL)applySettings:(id)settings;
- (CGDirectDisplayID)displayID;
@end

@implementation DFRemoteVirtualDisplay {
    id _display;
}

static BOOL supports(Class cls, NSArray<NSString *> *selectors) {
    if (!cls) return NO;
    for (NSString *name in selectors) {
        if (![cls instancesRespondToSelector:NSSelectorFromString(name)]) return NO;
    }
    return YES;
}

- (nullable instancetype)initWithSerial:(uint32_t)serial error:(NSError **)error {
    NSAssert([NSThread isMainThread], @"Virtual displays belong to the main thread");
    self = [super init];
    if (!self) return nil;
    Class descriptorClass = NSClassFromString(@"CGVirtualDisplayDescriptor");
    Class settingsClass = NSClassFromString(@"CGVirtualDisplaySettings");
    Class modeClass = NSClassFromString(@"CGVirtualDisplayMode");
    Class displayClass = NSClassFromString(@"CGVirtualDisplay");
    NSString *reason = @"This macOS version does not support DeskFuse virtual displays";
    if (supports(descriptorClass, @[@"setName:", @"setQueue:", @"setMaxPixelsWide:",
            @"setMaxPixelsHigh:", @"setSizeInMillimeters:", @"setVendorID:", @"setProductID:", @"setSerialNum:",
            @"setRedPrimary:", @"setGreenPrimary:", @"setBluePrimary:", @"setWhitePoint:"])
        && supports(settingsClass, @[@"setHiDPI:", @"setModes:"])
        && supports(modeClass, @[@"initWithWidth:height:refreshRate:"])
        && supports(displayClass, @[@"initWithDescriptor:", @"applySettings:", @"displayID"])) {
        @try {
            id descriptor = [[descriptorClass alloc] init];
            [descriptor setName:@"DeskFuse Remote Application"];
            [descriptor setQueue:dispatch_get_main_queue()];
            [descriptor setMaxPixelsWide:1920];
            [descriptor setMaxPixelsHigh:1080];
            [descriptor setSizeInMillimeters:CGSizeMake(508, 285.75)];
            [descriptor setRedPrimary:CGPointMake(0.64, 0.33)];
            [descriptor setGreenPrimary:CGPointMake(0.30, 0.60)];
            [descriptor setBluePrimary:CGPointMake(0.15, 0.06)];
            [descriptor setWhitePoint:CGPointMake(0.3127, 0.3290)];
            // Keep in sync with deskfuse-platform-macos/src/display.rs. The
            // signed input service recognizes these without shared UI state.
            [descriptor setVendorID:0x4453];
            [descriptor setProductID:0x5241];
            [descriptor setSerialNum:serial];
            id settings = [[settingsClass alloc] init];
            [settings setHiDPI:0];
            id mode = [[modeClass alloc] initWithWidth:1920 height:1080 refreshRate:60.0];
            if (mode) {
                [settings setModes:@[mode]];
                _display = [[displayClass alloc] initWithDescriptor:descriptor];
                if (_display && [_display applySettings:settings] && [_display displayID] != 0) return self;
            }
            reason = @"WindowServer could not create a 1920x1080 virtual display";
        } @catch (NSException *exception) {
            reason = @"The macOS virtual display API is incompatible";
        }
    }
    _display = nil;
    if (error) *error = [NSError errorWithDomain:@"DeskFuse.VirtualDisplay" code:1
        userInfo:@{NSLocalizedDescriptionKey: reason}];
    return nil;
}

- (CGDirectDisplayID)displayID { return [_display displayID]; }
@end
