-- Minimal Halo event/display loop. Transport, consent, and policy stay in host Rust.
local MAX_CELLS = 1024
local state = { active = false, connected = false }

local function emit(kind, payload)
  if not state.active or not state.connected then return false end
  halo.send_event(kind, payload)
  return true
end

function on_start()
  state.active = true
  halo.on_button(function(name, pressed) emit("button", { name = name, pressed = pressed }) end)
  halo.on_sensor(function(name, value_milli) emit("sensor", { name = name, value_milli = value_milli }) end)
  halo.on_link(function(connected) state.connected = connected; emit("link-state", { connected = connected }) end)
end

function on_display(cells)
  if not state.active or not state.connected or type(cells) ~= "string" or #cells > MAX_CELLS then return false end
  halo.display_cells(cells)
  return true
end

function on_clear()
  if state.active then halo.clear_display() end
end

function on_suspend()
  state.active = false
  state.connected = false
  halo.clear_display()
end
