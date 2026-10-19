!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $DeleteAppDataCheckboxState = 1
  ${AndIf} $UpdateMode <> 1
    SetShellVarContext current
    RmDir /r "$APPDATA\cryptoupload"
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ${IfNot} ${Silent}
  ${AndIf} $PassiveMode <> 1
  ${AndIf} $UpdateMode <> 1
    StrCpy $0 ""
    SetRegView 64
    ReadRegStr $0 HKLM "SOFTWARE\WinFsp" "InstallDir"
    ${If} $0 == ""
      ReadRegStr $0 HKLM "SOFTWARE\WOW6432Node\WinFsp" "InstallDir"
    ${EndIf}
    ${If} $0 == ""
      SetRegView 32
      ReadRegStr $0 HKLM "SOFTWARE\WinFsp" "InstallDir"
    ${EndIf}
    SetRegView 64
    ${If} $0 == ""
      MessageBox MB_YESNO|MB_ICONINFORMATION "$(CuWinfspPrompt)" IDNO cu_winfsp_done
      ExecShell "open" "https://winfsp.dev/rel/"
      cu_winfsp_done:
    ${EndIf}
  ${EndIf}
!macroend
