import { EmptyState, Link, Text } from '@hubspot/ui-extensions';
import {
  hubspot,
  ExtensionPointApiActions,
  SettingsContext,
} from '@hubspot/ui-extensions';

interface SettingsExtensionProps {
  context: SettingsContext;
  actions: ExtensionPointApiActions<'settings'>;
}

hubspot.extend<'settings'>(({ context, actions }: SettingsExtensionProps) => (
  <SettingsPage context={context} actions={actions} />
));

const SettingsPage = ({ context: _context }: SettingsExtensionProps) => (
  <EmptyState
    title="SagePAS integration"
    layout="horizontal"
    imageName="settings"
  >
    <Text>
      SagePAS is the SageSure-US Policy workspace for US customer, quote,
      policy, renewal, payment, claim, document, and agent operations. HubSpot
      is the CRM and integration layer; it is not represented as a product.
    </Text>
    <Link href="https://pas.sagesure.io/">Open SagePAS</Link>
  </EmptyState>
);
