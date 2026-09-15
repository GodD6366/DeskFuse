-- Read-only. The app calls readDestination on a native blocking worker.
-- Running this source with osascript queries the front Finder window directly.
on run
    return my readDestination(false)
end run

on readDestination(useDesktop)
    try
        with timeout of 2 seconds
            tell application id "com.apple.finder"
                if useDesktop then
                    return {0, 0, POSIX path of (desktop as alias)}
                end if
                if (count of Finder windows) is 0 then return {-2702, 0, ""}
                set originalWindow to front Finder window
                set originalID to id of originalWindow
                set originalPath to POSIX path of (target of originalWindow as alias)
                return {0, originalID, originalPath}
            end tell
        end timeout
    on error number errorNumber
        -- Never return Finder's error text: it can contain user paths/names.
        return {errorNumber, 0, ""}
    end try
end readDestination
