package ai.arcships.aimux;

import java.io.PrintWriter;
import org.junit.platform.launcher.Launcher;
import org.junit.platform.launcher.core.LauncherFactory;
import org.junit.platform.launcher.core.LauncherDiscoveryRequestBuilder;
import org.junit.platform.launcher.listeners.SummaryGeneratingListener;
import org.junit.platform.engine.discovery.DiscoverySelectors;
public final class ContractTestRunner {
    public static void main(String[] args) {
        Launcher launcher = LauncherFactory.create();
        SummaryGeneratingListener listener = new SummaryGeneratingListener();
        launcher.registerTestExecutionListeners(listener);
        launcher.execute(LauncherDiscoveryRequestBuilder.request()
            .selectors(DiscoverySelectors.selectPackage("ai.arcships.aimux")).build());
        listener.getSummary().printTo(new PrintWriter(System.out));
        listener.getSummary().printFailuresTo(new PrintWriter(System.out));
        if (listener.getSummary().getTestsFailedCount() > 0 || listener.getSummary().getTestsSucceededCount() == 0) {
            throw new AssertionError("JUnit tests failed or no tests ran");
        }
    }
}
