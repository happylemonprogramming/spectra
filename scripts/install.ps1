# Spectra for Windows, in one line, once it runs there:
#
#   irm https://__SPECTRA_RAW_URL__/scripts/install.ps1 | iex
#
# Spectra runs on Linux for now: Windows needs a drive backend of its own
# (SPTI), and macOS one through IOKit. See PLAN.md, "Later: macOS and Windows".
# No `exit` here: under `iex` it would close the PowerShell window.
Write-Host "Spectra runs on Linux for now. Windows support is planned: see PLAN.md, 'Later: macOS and Windows'." -ForegroundColor Yellow
