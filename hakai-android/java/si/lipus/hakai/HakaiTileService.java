package si.lipus.hakai;

import android.app.NativeActivity;
import android.app.PendingIntent;
import android.content.Intent;
import android.os.Build;
import android.service.quicksettings.Tile;
import android.service.quicksettings.TileService;

/**
 * The "Hakai" Quick Settings tile — the only Java in hakai.
 *
 * Launching hakai from the home screen puts the launcher behind its translucent window, so
 * that's what gets smashed. The notification shade can be pulled down over any app; a tap
 * here starts hakai on top of that app instead, collapsing the shade, so the app stays
 * visible underneath.
 */
public class HakaiTileService extends TileService {
    @Override
    public void onStartListening() {
        Tile tile = getQsTile();
        if (tile != null) {
            // A launcher, not a toggle: always shown in the neutral state.
            tile.setState(Tile.STATE_INACTIVE);
            tile.updateTile();
        }
    }

    @Override
    @SuppressWarnings("deprecation") // the Intent overload, for Android 7–13
    public void onClick() {
        Intent intent = new Intent(this, NativeActivity.class).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
        if (Build.VERSION.SDK_INT >= 34) {
            // Android 14+ only accepts a PendingIntent here.
            PendingIntent pending = PendingIntent.getActivity(
                    this, 0, intent, PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
            startActivityAndCollapse(pending);
        } else {
            startActivityAndCollapse(intent);
        }
    }
}
