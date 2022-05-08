:: Make sure the Service has been stopped
sc.exe stop FixFirefoxLauncher 4:1:2 "Uninstalling service"
:: Now remove it
sc.exe delete FixFirefoxLauncher
