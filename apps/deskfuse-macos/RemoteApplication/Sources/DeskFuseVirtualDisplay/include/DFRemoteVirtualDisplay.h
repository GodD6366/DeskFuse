#import <Foundation/Foundation.h>
#import <CoreGraphics/CoreGraphics.h>

NS_ASSUME_NONNULL_BEGIN

/// Runtime-only bridge to the undocumented CGVirtualDisplay API. Must be used
/// on the main thread. Releasing this owner removes the display.
@interface DFRemoteVirtualDisplay : NSObject
@property(nonatomic, readonly) CGDirectDisplayID displayID;
- (nullable instancetype)initWithSerial:(uint32_t)serial error:(NSError **)error;
@end

NS_ASSUME_NONNULL_END
