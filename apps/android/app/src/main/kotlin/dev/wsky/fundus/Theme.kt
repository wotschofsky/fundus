package dev.wsky.fundus

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.MaterialExpressiveTheme
import androidx.compose.material3.MotionScheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color

// Fundus's neutral palette (DESIGN.md: white ground, neutral ink, red only for problems)
// mapped onto Material 3 roles, so every stock component picks it up.

private val Light = lightColorScheme(
    primary = Color(0xFF18181B), onPrimary = Color.White,
    primaryContainer = Color(0xFFE4E4E7), onPrimaryContainer = Color(0xFF18181B),
    secondary = Color(0xFF52525B), onSecondary = Color.White,
    secondaryContainer = Color(0xFFEDEDF0), onSecondaryContainer = Color(0xFF18181B),
    tertiary = Color(0xFF3F3F46), onTertiary = Color.White,
    tertiaryContainer = Color(0xFFE4E4E7), onTertiaryContainer = Color(0xFF18181B),
    background = Color.White, onBackground = Color(0xFF18181B),
    surface = Color.White, onSurface = Color(0xFF18181B),
    surfaceVariant = Color(0xFFF1F1F3), onSurfaceVariant = Color(0xFF5B5B64),
    surfaceTint = Color(0xFF18181B),
    inverseSurface = Color(0xFF27272A), inverseOnSurface = Color(0xFFF4F4F5), inversePrimary = Color(0xFFD4D4D8),
    error = Color(0xFFB42318), onError = Color.White,
    errorContainer = Color(0xFFFDECEA), onErrorContainer = Color(0xFF7A1A11),
    outline = Color(0xFFC9C9D0), outlineVariant = Color(0xFFE8E8EC),
    surfaceBright = Color.White, surfaceDim = Color(0xFFE4E4E7),
    surfaceContainerLowest = Color.White, surfaceContainerLow = Color(0xFFFAFAFA),
    surfaceContainer = Color(0xFFF5F5F6), surfaceContainerHigh = Color(0xFFEFEFF1),
    surfaceContainerHighest = Color(0xFFE8E8EC),
)

private val Dark = darkColorScheme(
    primary = Color(0xFFF4F4F5), onPrimary = Color(0xFF18181B),
    primaryContainer = Color(0xFF3F3F46), onPrimaryContainer = Color(0xFFF4F4F5),
    secondary = Color(0xFFA1A1AA), onSecondary = Color(0xFF18181B),
    secondaryContainer = Color(0xFF2E2E33), onSecondaryContainer = Color(0xFFF4F4F5),
    tertiary = Color(0xFFD4D4D8), onTertiary = Color(0xFF18181B),
    tertiaryContainer = Color(0xFF3F3F46), onTertiaryContainer = Color(0xFFF4F4F5),
    background = Color(0xFF111113), onBackground = Color(0xFFF4F4F5),
    surface = Color(0xFF111113), onSurface = Color(0xFFF4F4F5),
    surfaceVariant = Color(0xFF27272A), onSurfaceVariant = Color(0xFFA1A1AA),
    surfaceTint = Color(0xFFF4F4F5),
    inverseSurface = Color(0xFFE4E4E7), inverseOnSurface = Color(0xFF18181B), inversePrimary = Color(0xFF3F3F46),
    error = Color(0xFFF97066), onError = Color(0xFF450A0A),
    errorContainer = Color(0xFF5C1A14), onErrorContainer = Color(0xFFFECDCA),
    outline = Color(0xFF52525B), outlineVariant = Color(0xFF2E2E33),
    surfaceBright = Color(0xFF2E2E33), surfaceDim = Color(0xFF111113),
    surfaceContainerLowest = Color(0xFF0C0C0E), surfaceContainerLow = Color(0xFF18181B),
    surfaceContainer = Color(0xFF1C1C1F), surfaceContainerHigh = Color(0xFF242428),
    surfaceContainerHighest = Color(0xFF2E2E33),
)

/** Material 3 Expressive: the neutral palette with expressive (springy) motion throughout. */
@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun FundusTheme(content: @Composable () -> Unit) {
    MaterialExpressiveTheme(
        colorScheme = if (isSystemInDarkTheme()) Dark else Light,
        motionScheme = MotionScheme.expressive(),
        content = content,
    )
}
