!macro customUninstall
  ; Delete application data in AppData/Roaming and AppData/Local
  RMDir /r "$APPDATA\kx-player"
  RMDir /r "$LOCALAPPDATA\kx-player"

  ; Delete any background image stored alongside the old executable (legacy location)
  ; This line is harmless if the file does not exist
  Delete "$INSTDIR\kx-player-bg.png"

  ; Delete registry entries created by the application or installer
  DeleteRegKey HKCU "Software\kx-player"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\com.kxplayer.music"
  DeleteRegKey HKCU "Software\Classes\com.kxplayer.music"
!macroend
