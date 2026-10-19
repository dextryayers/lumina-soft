local mp = require("mp")
local options = require("mp.options")

local config = {
    lumina_send = "lumina-send",
    notify_pause = true,
}

options.read_options(config, "lumina")

local function notify(title, body)
    mp.command_native_async({
        name = "subprocess",
        args = {
            config.lumina_send,
            "notify",
            "--title",
            title,
            "--body",
            body,
            "--duration-ms",
            "4000",
        },
    }, function() end)
end

mp.register_event("file-loaded", function()
    local title = mp.get_property("media-title")
        or mp.get_property("filename")
        or "unknown"
    notify("mpv", "Playing: " .. title)
end)

local last_pause = nil

mp.observe_property("pause", "bool", function(_, paused)
    if paused == nil then
        return
    end
    if config.notify_pause and last_pause ~= nil and paused ~= last_pause then
        notify("mpv", paused and "Paused" or "Resumed")
    end
    last_pause = paused
end)
