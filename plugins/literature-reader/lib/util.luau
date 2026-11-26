--!nocheck
-- Small, dependency-free helpers shared by the host half.
local tern = require("tern")

local M = {}

local STATE_CAP = 32 * 1024 * 1024

function M.exists(path)
	if type(path) ~= "string" or path == "" then
		return false
	end
	local ok, value = pcall(tern.fs.exists, path)
	return ok and value == true
end

-- Raw bytes, or nil. Uses a bound so a runaway writer cannot exhaust memory.
function M.readBytes(path, cap)
	if type(path) ~= "string" or path == "" then
		return nil
	end
	local ok, value = pcall(tern.fs.read, path, cap or STATE_CAP)
	if ok and type(value) == "string" then
		return value
	end
	return nil
end

function M.readText(path, cap)
	return M.readBytes(path, cap)
end

function M.decode(text)
	if type(text) ~= "string" or text == "" then
		return nil
	end
	local ok, value = pcall(tern.json.decode, text)
	if ok then
		return value
	end
	return nil
end

function M.readJson(path, cap)
	local text = M.readBytes(path, cap)
	if not text then
		return nil
	end
	return M.decode(text)
end

function M.remove(path)
	if type(path) ~= "string" or path == "" then
		return
	end
	pcall(tern.fs.remove, path)
end

function M.basename(path)
	if type(path) ~= "string" or path == "" then
		return ""
	end
	local name = string.match(path, "[^\\/]+$") or path
	return name
end

-- Cut a UTF-8 string at n characters without splitting a code point.
function M.truncate(text, n)
	if type(text) ~= "string" then
		return ""
	end
	local count = utf8.len(text)
	if count and count <= n then
		return text
	end
	if count then
		local ok, offset = pcall(utf8.offset, text, n + 1)
		if ok and offset and offset > 1 then
			return string.sub(text, 1, offset - 1) .. "…"
		end
	end
	return string.sub(text, 1, n) .. "…"
end

function M.format1(value)
	if type(value) ~= "number" then
		return nil
	end
	return string.format("%.1f", value)
end

function M.clamp(value, low, high)
	if value < low then
		return low
	end
	if value > high then
		return high
	end
	return value
end

return M
