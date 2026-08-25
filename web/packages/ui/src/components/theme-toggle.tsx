import { Moon, Sun } from "lucide-react"
import {Button} from "@/components/ui/button"
import { useTheme } from "@/hooks/use-theme"

export function ThemeToggle() {
  const { isDark, toggleTheme } = useTheme()

  return (
    <Button variant="ghost" size="icon" aria-label={isDark ? "切换为亮色主题" : "切换为暗色主题"} onClick={toggleTheme}>
      {isDark ? <Sun /> : <Moon />}
    </Button>
  )
}

