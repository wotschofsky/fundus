package dev.wsky.fundus

import android.app.Application
import android.content.Context
import android.content.Intent
import android.os.Bundle
import android.text.format.DateFormat
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.animation.EnterTransition
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.scaleOut
import androidx.compose.material3.MaterialTheme
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.dp
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.material3.SnackbarDuration
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.SnackbarResult
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.platform.LocalContext
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.lifecycleScope
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.rememberNavController
import androidx.navigation.toRoute
import kotlinx.coroutines.launch
import kotlinx.serialization.Serializable

class FundusApp : Application() {
    val vault by lazy { Vault(this) }
    val library by lazy { Library(vault) }
}

val Context.fundus get() = applicationContext as FundusApp

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        if (savedInstanceState == null) handle(intent)
        setContent { FundusTheme { FundusRoot() } }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handle(intent)
    }

    override fun onResume() {
        super.onResume()
        Format.hour24 = DateFormat.is24HourFormat(this)
    }

    /** The browser hands the vault's approval back as `dev.wsky.fundus:/oauth?code=…`. */
    private fun handle(intent: Intent?) {
        val callback = intent?.data?.takeIf { it.scheme == "dev.wsky.fundus" } ?: return
        lifecycleScope.launch { fundus.vault.finishSignIn(callback.toString()) }
    }
}

@Serializable private object Recordings
@Serializable private data class Recording(val id: String, val at: Long? = null)

@Composable
private fun FundusRoot() {
    val app = LocalContext.current.fundus
    val server by app.vault.server.collectAsStateWithLifecycle()
    if (server == null) return ConnectScreen(app.vault)

    val navigation = rememberNavController()
    val snackbar = remember { SnackbarHostState() }
    // Outlives either screen, so a recording trashed from its own page can still be restored.
    val scope = rememberCoroutineScope()
    val trash: (Entry) -> Unit = { entry ->
        scope.launch {
            try {
                app.library.perform(Library.Action.Trash, entry.id)
            } catch (error: Exception) {
                snackbar.showSnackbar(error.describe())
                return@launch
            }
            val undo = snackbar.showSnackbar("Moved to trash", actionLabel = "Undo", duration = SnackbarDuration.Long)
            if (undo == SnackbarResult.ActionPerformed) {
                runCatching { app.library.perform(Library.Action.Restore, entry.id) }
                    .onFailure { snackbar.showSnackbar(it.describe()) }
            }
        }
    }

    val motion = MaterialTheme.motionScheme
    val slide = with(LocalDensity.current) { 30.dp.roundToPx() }
    NavHost(
        navController = navigation,
        startDestination = Recordings,
        // Material's shared axis: the old page fades out before the new one fades in, so the two
        // never overlap, while both move a short way on the theme's spring.
        enterTransition = { slideInHorizontally(motion.defaultSpatialSpec()) { slide } + fadeIn(tween(210, delayMillis = 90)) },
        exitTransition = { slideOutHorizontally(motion.defaultSpatialSpec()) { -slide } + fadeOut(tween(90)) },
        // Back, by button or gesture: the page shrinks away over the one beneath, as in Android's own
        // predictive back. The gesture drives it, so it follows the finger.
        popEnterTransition = { EnterTransition.None },
        popExitTransition = { scaleOut(motion.defaultSpatialSpec(), targetScale = 0.9f) + fadeOut(motion.defaultEffectsSpec()) },
    ) {
        composable<Recordings> {
            LibraryScreen(
                library = app.library,
                vault = app.vault,
                snackbar = snackbar,
                onOpen = { id, at -> navigation.navigate(Recording(id, at)) },
                onTrash = trash,
            )
        }
        composable<Recording> { entry ->
            val route = entry.toRoute<Recording>()
            val model = viewModel { TranscriptModel(app, route.id, route.at) }
            TranscriptScreen(
                model = model,
                library = app.library,
                snackbar = snackbar,
                onBack = { navigation.popBackStack() },
                onTrash = {
                    navigation.popBackStack()
                    trash(it)
                },
            )
        }
    }
}
