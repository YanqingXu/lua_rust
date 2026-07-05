integration_helper_runs = (integration_helper_runs or 0) + 1
integration_helper_message = "helper-run-" .. integration_helper_runs

print("Running helper workflow chunk: " .. integration_helper_message)

function integration_helper_join(left, right)
    return left .. ":" .. right, left .. right
end

integration_helper_table = {
    n = 3,
    integration_helper_runs,
    integration_helper_runs + 10,
    integration_helper_runs + 20
}


print("Helper workflow chunk completed: " .. integration_helper_table.n .. " entries, message: ")