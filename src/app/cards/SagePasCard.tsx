import {
  CrmContext,
  EmptyState,
  ExtensionPointApiActions,
  Link,
  Text,
} from '@hubspot/ui-extensions';
import { hubspot } from '@hubspot/ui-extensions';

interface CrmExtensionProps {
  context: CrmContext;
  actions: ExtensionPointApiActions<'crm.record.tab'>;
}

hubspot.extend<'crm.record.tab'>(({ context, actions }: CrmExtensionProps) => (
  <SagePasCard context={context} actions={actions} />
));

const SagePasCard = ({ context }: CrmExtensionProps) => {
  const recordId = context.crm.objectId;
  const portalId = context.portal.id;
  const pasUrl = `https://pas.sagesure.io/quotes?hubspotPortalId=${encodeURIComponent(portalId)}&hubspotContactId=${encodeURIComponent(recordId)}`;

  return (
    <EmptyState
      title="SagePAS Contact Customer and Quote"
      layout="vertical"
      imageName="documents"
    >
      <Text>
        Resolve this contact to its native SagePAS customer, link the customer
        deliberately when needed, and start or reopen the contact&apos;s quote.
      </Text>
      <Link href={pasUrl}>Open contact quote workflow in SagePAS</Link>
    </EmptyState>
  );
};
