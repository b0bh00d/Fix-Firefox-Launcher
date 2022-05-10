<p align="center">
  <a href="https://www.mozilla.org/en-US/firefox/new/">
    <img width="20%" alt="FixFirefoxLauncher" src="https://cdn.freebiesupply.com/logos/large/2x/firefox-logo-png-transparent.png">
  </a>
</p>

# Fix Firefox Launcher
A Windows service to keep Firefox from replacing custom launch options.

## Summary
This project is an extension of [my preivous work](https://github.com/b0bh00d/Fix-Chrome-Launcher) to address similar behavior in Chrome.  You can review the rationale there; I'll only talk about the variations here.

Clicking (or right-clicking) links anywhere triggers the use of a specific launcher string for the Firefox browser.  This string can be found in a pair of Windows registry values under the paths

- `HKEY_CLASSES_ROOT\FirefoxHTML_<id>\shell\open\command`
- `HKEY_CLASSES_ROOT\FirefoxURL_<id>\shell\open\command`

The default entries for these keys hold the command that Windows will execute when following links you click.

Every time Firefox does any kind of update, it usually resets these strings, and then I'm opening browser windows in standard "tracking" mode.  Granted, Firefox is better at privacy than Chrome, but I still do not like having cookies hanging around.

## Specifying options

### Within the Registry
"Fix Firefox Launcher" monitors these registry keys, and if it detects the launch string is missing the customized options, it will restore the custom options to both launcher commands values.

Options can be provided in one of two ways.  First, the custom options you want to use in the launcher string can be placed in the same registry key.  A new REG_SZ value called `ffl_options` will hold the options (and their attendant args) that need to be reapplied.  For this application, the options you provide will completely replace those found in the default command--this means you will need to include (append) the token replacement value "%1" in your custom options.  For example, your `ffl_options` REG_SZ value might contain:

`-private-window "%1"`

Optionally, you can add another entry to this registry key called `ffl_interval`.  This is a REG_DWORD value that holds the specific polling interval you would like the service to use when checking the integrity of the launcher string.  By default, the service checks every 60 seconds. You can override this with a new value in this numeric entry.

### As Service Options
You may also provide runtime arguments from within the Windows Service panel if you manually "Start" the process.  The option names are identical to those in the registry, but are formatted like command-line arguments.  For example:

`--ffl_interval=10 --ffl_options="-private-window \"%1\""`

Arguments provide within the Service panel take higher precedence than those in the Registry, and will replace the corresponding values if you use both.

## Building

On Windows, simply compile with: `cargo build --release`

## Installing

Since this is a Windows Service, you will need to open a command window (with elevated privileges) to install or uninstall it.  I have included DOS batch files in the repository with commands for manually installing and removing the Service.  They largely contain the commands listed below.

To install:

`sc.exe create FixFirefoxLauncher binPath= "<path_to>\fix_firefox_launcher.exe" DisplayName= "Fix Firefox Launcher"`

To uninstall, first make sure the Windows Service is not running:

`sc.exe stop FixFirefoxLauncher 4:1:2 "Uninstalling service"`

Then actually remove the Service from the system:

`sc.exe delete FixFirefoxLauncher`

## Running

You can start/stop the service from the command line, or you can configure it to run automatically from the Windows Services panel (recommended).  Operating the Service from the "Local System" will likely not be sufficient to allow the process to read/modify the Registry entries.  You can test that first, but the best approach will be to log the Service in using your regular Windows account credentials.

The service emits messages to the system console, so you can check there (i.e., `Event Viewer` -> `Windows Logs` -> `Application`) for any runtime error messages.  Look for the SourceName "FixFirefoxLauncher".

I hope you find this useful.